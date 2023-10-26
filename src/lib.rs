use std::{
    cell::RefCell,
    panic::{catch_unwind, panic_any, resume_unwind, AssertUnwindSafe},
    sync::atomic::{AtomicU64, Ordering},
};

use proc_macro2::{Delimiter, Ident, Span, TokenStream, TokenTree};
use quote::{quote, ToTokens};

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

pub fn import(path: impl ToTokens) -> LazyTokenStream {
    IMPORT_SESSION.with(|session| {
        let mut session = session.borrow_mut();
        let session = session
            .as_mut()
            .expect("can only `import` in a `begin_import` closure");

        session.requests.push(path.into_token_stream());

        LazyTokenStream(
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
pub struct LazyTokenStream(Option<TokenStream>);

impl LazyTokenStream {
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
