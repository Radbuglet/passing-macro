use std::{
    cell::{OnceCell, RefCell},
    mem,
    ops::{Deref, DerefMut},
    panic::{catch_unwind, panic_any, resume_unwind, AssertUnwindSafe},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use proc_macro2::{Delimiter, Ident, Span, TokenStream, TokenTree};
use quote::{quote, ToTokens};

// === Core === //

struct MissingContextPanicReason;

thread_local! {
    static IMPORT_SESSION: RefCell<Option<ImportContextSession>> = RefCell::new(None);
}

struct ImportContextSession {
    requests: Vec<TokenStream>,
    inputs: Vec<TokenStream>,
    inputs_counter: usize,
}

pub fn begin_import(
    me: impl ToTokens,
    input: impl Into<TokenStream>,
    f: impl FnOnce(TokenStream) -> TokenStream,
) -> TokenStream {
    let mut input = input.into().into_iter();

    // Begin the session
    IMPORT_SESSION.with(|session| {
        let mut session = session.borrow_mut();

        // Ensure that this was not called reentrantly
        assert!(session.is_none(), "cannot nest begin_import calls");

        // Collect the received inputs
        let mut inputs = Vec::new();

        'parse: {
            // Attempt to parse the prefix
            {
                let mut fork_input = input.clone();
                match fork_input.next() {
                    Some(TokenTree::Ident(ident)) if ident == "__import_context_passer" => {}
                    _ => break 'parse,
                }
                input = fork_input;
            }

            let mut input = match input.next() {
                Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Bracket => {
                    group.stream().into_iter()
                }
                _ => panic!("invalid __import_context_passer syntax"),
            };

            loop {
                match input.next() {
                    Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => {
                        inputs.push(group.stream())
                    }
                    None => break,
                    _ => panic!("invalid __import_context_passer syntax"),
                }
            }
        }

        *session = Some(ImportContextSession {
            requests: Vec::new(),
            inputs,
            inputs_counter: 0,
        });
    });

    let input: TokenStream = input.collect();

    // Attempt to complete the macro invocation
    catch_unwind(AssertUnwindSafe(|| {
        let output = f(input.clone());
        IMPORT_SESSION.with(|session| session.borrow_mut().take());
        output
    }))
    // However, if it failed...
    .unwrap_or_else(|err| {
        // Take out the session.
        let session = IMPORT_SESSION
            .with(|session| session.borrow_mut().take())
            .unwrap();

        // If the reason for panic was missing context...
        if err.downcast_ref::<MissingContextPanicReason>().is_some() {
            // Request more context with which to rerun the macro...
            let inputs = &session.inputs;
            let (first, requests) = session.requests[session.inputs_counter..]
                .split_first()
                // This panic couldn't have triggered without requesting at least one additional
                // input.
                .unwrap();

            quote! {
                #first! {
                    __import_context_passer [
                        #({ #inputs })*
                        #((#requests))* (#me)
                    ]
                    #input
                }
            }
        } else {
            // Otherwise, propagate the panic.
            resume_unwind(err);
        }
    })
}

pub fn import(path: impl ToTokens) -> LazyImport {
    IMPORT_SESSION.with(|session| {
        let mut session = session.borrow_mut();
        let session = session
            .as_mut()
            .expect("can only `import` in a `begin_import` closure");

        session.requests.push(path.into_token_stream());

        LazyImport(
            if let Some(input) = session.inputs.get(session.inputs_counter) {
                session.inputs_counter += 1;
                Some(input.clone())
            } else {
                None
            },
        )
    })
}

#[derive(Debug, Clone)]
pub struct LazyImport(Option<TokenStream>);

impl LazyImport {
    pub fn clone_eval(&self) -> TokenStream {
        self.clone().eval()
    }

    pub fn eval(self) -> TokenStream {
        self.0
            .unwrap_or_else(|| panic_any(MissingContextPanicReason))
    }
}

pub fn export(
    vis: impl ToTokens,
    name: impl ToTokens,
    data: impl ToTokens,
) -> (Ident, TokenStream) {
    let id = unique_ident(Span::mixed_site());

    let stream = quote! {
        #[macro_export]
        #[doc(hidden)]
        macro_rules! #id {
            (
                __import_context_passer [
                    $({ $($input:tt)* })*
                    ($next_path:path) $(($later_path:path))*
                ]
                $($args:tt)*
            ) => {
                $next_path! {
                    __import_context_passer [
                        $({ $($input)* })* { #data }
                        $(($later_path))*
                    ]
                    $($args)*
                }
            };
            ($($tt:tt)*) => { ::core::compile_error!("cannot call this item directly as a macro") };
        }

        #[doc(hidden)]
        #vis use #id as #name;
    };

    (id, stream)
}

pub fn unique_ident(span: Span) -> Ident {
    const COMPILATION_TAG: u64 = const_random::const_random!(u64);
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    Ident::new(
        &format!(
            "__unique_ident_{COMPILATION_TAG}_{}",
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ),
        span,
    )
}

// === Helpers === //

// Internal Processor
struct ConcreteLazyProcessor<T, E, F>
where
    F: FnOnce(TokenStream) -> Result<T, E>,
{
    inner: RefCell<ConcreteLazyProcessorInner<T, E, F>>,
}

enum ConcreteLazyProcessorInner<T, E, F>
where
    F: FnOnce(TokenStream) -> Result<T, E>,
{
    NotComputed(LazyImport, F),
    Computed(Option<T>),
}

trait SpecificLazyProcessor {
    type Output;

    fn steal(&self) -> Self::Output;
}

impl<T, E, F> SpecificLazyProcessor for ConcreteLazyProcessor<T, E, F>
where
    F: FnOnce(TokenStream) -> Result<T, E>,
{
    type Output = T;

    fn steal(&self) -> Self::Output {
        let inner = &mut *self.inner.borrow_mut();

        match inner {
            ConcreteLazyProcessorInner::NotComputed(_, _) => panic!(
                "the entire LazyImportGroup must be evaluated before dereferencing a \
				 ComputedLazyImport",
            ),
            ConcreteLazyProcessorInner::Computed(value) => value.take().unwrap(),
        }
    }
}

trait ErasedLazyProcessor {
    type Error;

    fn compute(&self) -> Result<(), Self::Error>;
}

impl<T, E, F> ErasedLazyProcessor for ConcreteLazyProcessor<T, E, F>
where
    F: FnOnce(TokenStream) -> Result<T, E>,
{
    type Error = E;

    fn compute(&self) -> Result<(), Self::Error> {
        let inner = &mut *self.inner.borrow_mut();

        match mem::replace(inner, ConcreteLazyProcessorInner::Computed(None)) {
            ConcreteLazyProcessorInner::NotComputed(tokens, executor) => {
                *inner = ConcreteLazyProcessorInner::Computed(Some(executor(tokens.eval())?));
                Ok(())
            }
            ConcreteLazyProcessorInner::Computed(_) => unreachable!(),
        }
    }
}

// LazyImportGroup
pub struct LazyImportGroup<'a, E> {
    items: Vec<Rc<dyn 'a + ErasedLazyProcessor<Error = E>>>,
}

impl<E> LazyImportGroup<'_, E> {
    pub const fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn eval(&mut self) -> Option<Vec<E>> {
        let errors = self
            .items
            .drain(..)
            .filter_map(|processor| processor.compute().err())
            .collect::<Vec<_>>();

        (!errors.is_empty()).then_some(errors)
    }
}

impl<'a, E: 'a> LazyImportGroup<'a, E> {
    pub fn import<T: 'a>(
        &mut self,
        path: impl ToTokens,
        f: impl 'a + FnOnce(TokenStream) -> Result<T, E>,
    ) -> ComputedLazyImport<'a, T> {
        let import = import(path);
        let processor = Rc::new(ConcreteLazyProcessor {
            inner: RefCell::new(ConcreteLazyProcessorInner::NotComputed(import, f)),
        });

        self.items
            .push(Rc::clone(&processor) as Rc<dyn ErasedLazyProcessor<Error = E>>);

        ComputedLazyImport {
            processor,
            value: OnceCell::new(),
        }
    }
}

impl<E> Default for LazyImportGroup<'_, E> {
    fn default() -> Self {
        Self::new()
    }
}

// ComputedLazyImport
pub struct ComputedLazyImport<'a, T> {
    processor: Rc<dyn 'a + SpecificLazyProcessor<Output = T>>,
    value: OnceCell<T>,
}

impl<T> ComputedLazyImport<'_, T> {
    fn init(&self) -> &T {
        self.value.get_or_init(|| self.processor.steal())
    }

    pub fn into_inner(self) -> T {
        self.init();
        self.value.into_inner().unwrap()
    }
}

impl<T> Deref for ComputedLazyImport<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.init()
    }
}

impl<T> DerefMut for ComputedLazyImport<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.init();
        self.value.get_mut().unwrap()
    }
}
