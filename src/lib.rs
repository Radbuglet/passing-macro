use std::{
    cell::{Cell, RefCell},
    future::Future,
    hash,
    pin::Pin,
    task::{Context, Poll},
};

use hashbrown::HashMap;
use proc_macro::TokenStream as NativeTokenStream;
use proc_macro2::{Ident, Span, TokenStream, TokenTree};
use quote::{quote, ToTokens};
use rustc_hash::FxHasher;

extern crate proc_macro;

mod smuggle;

// === Hashing === //

struct ConstSafeDefaultHasher<T>([T; 0]);

impl<T: hash::Hasher + Default> hash::BuildHasher for ConstSafeDefaultHasher<T> {
    type Hasher = T;

    fn build_hasher(&self) -> Self::Hasher {
        T::default()
    }
}

type FxHashMap<K, V> = HashMap<K, V, ConstSafeDefaultHasher<FxHasher>>;

const fn new_hash_map<K, V>() -> FxHashMap<K, V> {
    FxHashMap::with_hasher(ConstSafeDefaultHasher([]))
}

// === Session Manager === //

struct SessionManager {
    gen: Cell<u64>,
    sessions: RefCell<FxHashMap<u64, Session>>,
}

struct Session {
    future: RefCell<Pin<Box<dyn Future<Output = ()>>>>,
    next_emit_builder: RefCell<TokenStream>,
    state: RefCell<SessionState>,
}

enum SessionState {
    Idle,
    Requesting(TokenStream),
    Resolved(TokenStream),
}

fn use_session_mgr<R>(f: impl FnOnce(&SessionManager) -> R) -> R {
    thread_local! {
        static SESSION_MANAGER: SessionManager =
            const { SessionManager {
                gen: Cell::new(0),
                sessions: RefCell::new(new_hash_map()),
            } };
    }

    SESSION_MANAGER.with(|mgr| f(mgr))
}

// === Session ID passing === //

fn encode_session_id(v: u64) -> Ident {
    Ident::new(&format!("s{v}"), Span::call_site())
}

pub fn decode_session_id(i: &Ident) -> u64 {
    i.to_string()
        .strip_prefix('s')
        .unwrap()
        .parse::<u64>()
        .unwrap()
}

// === Main Functions === //

enum PassMacroArgs {
    Resume {
        session: u64,
        data: proc_macro2::token_stream::IntoIter,
    },
    Create {
        args: TokenStream,
    },
}

fn parse_pass_macro_args(input: TokenStream) -> PassMacroArgs {
    // Handle resume syntax
    'subsequent_call: {
        let mut input = input.clone().into_iter();

        // Expect `__passing_macro_export`
        match input.next() {
            Some(TokenTree::Ident(ident)) if ident == "__passing_macro_export" => {}
            _ => break 'subsequent_call,
        }

        // Expect a session ID
        let session_id = match input.next() {
            Some(TokenTree::Ident(ident)) => decode_session_id(&ident),
            _ => break 'subsequent_call,
        };

        // Expect a semicolon
        match input.next() {
            Some(TokenTree::Punct(punct)) if punct.as_char() == ';' => {}
            _ => break 'subsequent_call,
        };

        return PassMacroArgs::Resume {
            session: session_id,
            data: input,
        };
    };

    // Otherwise, handle create syntax
    PassMacroArgs::Create { args: input }
}

pub fn pass_macro<F>(
    me: NativeTokenStream,
    input: NativeTokenStream,
    f: impl 'static + Send + FnOnce(PassMacroSession, TokenStream) -> F,
) -> NativeTokenStream
where
    F: Future<Output = ()> + 'static,
{
    smuggle::smuggle([me, input], |[me, input]| {
        let input = parse_pass_macro_args(input);

        use_session_mgr(|mgr| {
            // Create a new session if we need to
            let session_id = match &input {
                PassMacroArgs::Resume { session, .. } => *session,
                PassMacroArgs::Create { args } => {
                    mgr.gen.set(mgr.gen.get() + 1);
                    let session_id = mgr.gen.get();
                    let future = f(PassMacroSession { id: session_id }, args.clone());
                    mgr.sessions.borrow_mut().insert(
                        session_id,
                        Session {
                            future: RefCell::new(Box::pin(future)),
                            next_emit_builder: RefCell::new(TokenStream::default()),
                            state: RefCell::new(SessionState::Idle),
                        },
                    );

                    session_id
                }
            };

            // Now, fetch that session
            let sessions = mgr.sessions.borrow();
            let session = sessions
                .get(&session_id)
                .expect("internal passing-macro error: session lost");

            // If this is a resumption call, mark the request as having been completed
            if let PassMacroArgs::Resume { data, .. } = input {
                let state = &mut *session.state.borrow_mut();
                match state {
                    SessionState::Idle | SessionState::Resolved(_) => panic!(
						"session was not expecting additional tokens; did passing-macro forget some \
						state?",
					),
                    SessionState::Requesting(_) => *state = SessionState::Resolved(data.collect()),
                }
            }

            // Poll the future
            let mut future = session.future.borrow_mut();
            match future
                .as_mut()
                .poll(&mut Context::from_waker(&dummy_waker::dummy_waker()))
            {
                // If the future has finished...
                Poll::Ready(()) => {
                    // Close the session
                    drop(future);
                    drop(sessions);
                    let session = mgr.sessions.borrow_mut().remove(&session_id).unwrap();

                    // And return the value
                    session.next_emit_builder.into_inner()
                }
                // Otherwise, figure out what it's requesting and service it.
                Poll::Pending => {
                    let state = &mut *session.state.borrow_mut();

                    match state {
                        SessionState::Idle | SessionState::Resolved(_) => {
                            panic!("the pass_macro future suspended without fetching any data")
                        }
                        SessionState::Requesting(path) => {
                            let session_id = encode_session_id(session_id);
                            let next_emit = session.next_emit_builder.take();
                            quote! { #path!(__passing_macro_export #me ; #session_id); #next_emit }
                        }
                    }
                }
            }
        })
    })
}

#[derive(Copy, Clone)]
pub struct PassMacroSession {
    id: u64,
}

impl PassMacroSession {
    pub fn emit(self, data: impl ToTokens) {
        use_session_mgr(|mgr| {
            let sessions = &*mgr.sessions.borrow();
            let session = sessions
                .get(&self.id)
                .expect("internal passing-macro error: session lost");

            session
                .next_emit_builder
                .borrow_mut()
                .extend(data.to_token_stream());
        });
    }

    pub async fn fetch(self, path: impl ToTokens) -> TokenStream {
        let path = path.into_token_stream();

        // Transition from `idle` to `requesting`
        use_session_mgr(|mgr| {
            let sessions = &*mgr.sessions.borrow();
            let session = sessions
                .get(&self.id)
                .expect("internal passing-macro error: session lost");

            let state = &mut *session.state.borrow_mut();

            match state {
                SessionState::Idle => *state = SessionState::Requesting(path),
                SessionState::Requesting(_) => {
                    panic!("this future is already waiting for a result")
                }
                SessionState::Resolved(_) => {
                    panic!("the original fetch future has not yet consumed its result")
                }
            }
        });

        std::future::poll_fn(move |_cx| {
            use_session_mgr(|mgr| {
                let sessions = &*mgr.sessions.borrow();
                let session = sessions
                    .get(&self.id)
                    .expect("internal passing-macro error: session lost");

                let state = &mut *session.state.borrow_mut();

                match state {
                    SessionState::Idle => unreachable!(),
                    SessionState::Requesting(_) => Poll::Pending,
                    SessionState::Resolved(data) => {
                        let data = data.clone();
                        *state = SessionState::Idle;
                        Poll::Ready(data)
                    }
                }
            })
        })
        .await
    }
}

pub fn export_data(vis: impl ToTokens, name: Ident, data: impl ToTokens) -> TokenStream {
    let id = use_session_mgr(|mgr| {
        mgr.gen.set(mgr.gen.get() + 1);
        mgr.gen.get()
    });
    let id = Ident::new(&format!("__passing_macro_{id}"), name.span());

    quote! {
        #[macro_export]
        #[doc(hidden)]
        macro_rules! #id {
            (__passing_macro_export $target:path ; $($prefix:tt)*) => {
                $target!(__passing_macro_export $($prefix)* ; #data);
            };
        }

        #vis use #id as #name;
    }
}
