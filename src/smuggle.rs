use std::{any::Any, panic, str::FromStr, sync::mpsc};

use proc_macro::{
    Delimiter as NativeDelimiter, Group as NativeGroup, Ident as NativeIdent,
    Literal as NativeLiteral, Punct as NativePunct, Spacing as NativeSpacing, Span as NativeSpan,
    TokenStream as NativeTokenStream, TokenTree as NativeTokenTree,
};
use proc_macro2::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};

// === Serialized Data Structures === //

pub struct SerializedTokenStream {
    pub tokens: Vec<SerializedTokenTree>,
}

pub enum SerializedTokenTree {
    Group(SerializedGroup),
    Ident(SerializedIdent),
    Punct(SerializedPunct),
    Literal(SerializedLiteral),
}

pub struct SerializedGroup {
    pub delimiter: SerializedDelimiter,
    pub stream: SerializedTokenStream,
    pub span: SerializedSpan,
}

pub struct SerializedIdent {
    pub sym: String,
    pub span: SerializedSpan,
}

pub struct SerializedPunct {
    pub ch: char,
    pub spacing: SerializedSpacing,
    pub span: SerializedSpan,
}

pub struct SerializedLiteral {
    pub repr: String,
    pub span: SerializedSpan,
}

pub enum SerializedDelimiter {
    Parenthesis,
    Brace,
    Bracket,
    None,
}

pub enum SerializedSpacing {
    Alone,
    Joint,
}

pub struct SerializedSpan {}

// === Serialization Routines === //

impl SerializedTokenStream {
    pub fn from_native(stream: NativeTokenStream) -> Self {
        Self {
            tokens: stream
                .into_iter()
                .map(SerializedTokenTree::from_native)
                .collect(),
        }
    }

    pub fn from_fallback(stream: TokenStream) -> Self {
        Self {
            tokens: stream
                .into_iter()
                .map(SerializedTokenTree::from_fallback)
                .collect(),
        }
    }

    pub fn into_native(self) -> NativeTokenStream {
        self.tokens
            .into_iter()
            .map(SerializedTokenTree::into_native)
            .collect()
    }

    pub fn into_fallback(self) -> TokenStream {
        self.tokens
            .into_iter()
            .map(SerializedTokenTree::into_fallback)
            .collect()
    }
}

impl SerializedTokenTree {
    pub fn from_native(tree: NativeTokenTree) -> Self {
        match tree {
            NativeTokenTree::Group(group) => Self::Group(SerializedGroup::from_native(group)),
            NativeTokenTree::Ident(ident) => Self::Ident(SerializedIdent::from_native(ident)),
            NativeTokenTree::Punct(punct) => Self::Punct(SerializedPunct::from_native(punct)),
            NativeTokenTree::Literal(literal) => {
                Self::Literal(SerializedLiteral::from_native(literal))
            }
        }
    }

    pub fn from_fallback(tree: TokenTree) -> Self {
        match tree {
            TokenTree::Group(group) => Self::Group(SerializedGroup::from_fallback(group)),
            TokenTree::Ident(ident) => Self::Ident(SerializedIdent::from_fallback(ident)),
            TokenTree::Punct(punct) => Self::Punct(SerializedPunct::from_fallback(punct)),
            TokenTree::Literal(literal) => Self::Literal(SerializedLiteral::from_fallback(literal)),
        }
    }

    pub fn into_native(self) -> NativeTokenTree {
        match self {
            Self::Group(group) => NativeTokenTree::Group(group.into_native()),
            Self::Ident(ident) => NativeTokenTree::Ident(ident.into_native()),
            Self::Punct(punct) => NativeTokenTree::Punct(punct.into_native()),
            Self::Literal(literal) => NativeTokenTree::Literal(literal.into_native()),
        }
    }

    pub fn into_fallback(self) -> TokenTree {
        match self {
            Self::Group(group) => TokenTree::Group(group.into_fallback()),
            Self::Ident(ident) => TokenTree::Ident(ident.into_fallback()),
            Self::Punct(punct) => TokenTree::Punct(punct.into_fallback()),
            Self::Literal(literal) => TokenTree::Literal(literal.into_fallback()),
        }
    }
}

impl SerializedGroup {
    pub fn from_native(group: NativeGroup) -> Self {
        Self {
            delimiter: SerializedDelimiter::from_native(group.delimiter()),
            stream: SerializedTokenStream::from_native(group.stream()),
            span: SerializedSpan::from_native(group.span()),
        }
    }

    pub fn from_fallback(group: Group) -> Self {
        Self {
            delimiter: SerializedDelimiter::from_fallback(group.delimiter()),
            stream: SerializedTokenStream::from_fallback(group.stream()),
            span: SerializedSpan::from_fallback(group.span()),
        }
    }

    pub fn into_native(self) -> NativeGroup {
        let mut group = NativeGroup::new(self.delimiter.into_native(), self.stream.into_native());
        group.set_span(self.span.into_native());
        group
    }

    pub fn into_fallback(self) -> Group {
        let mut group = Group::new(self.delimiter.into_fallback(), self.stream.into_fallback());
        group.set_span(self.span.into_fallback());
        group
    }
}

impl SerializedIdent {
    pub fn from_native(ident: NativeIdent) -> Self {
        Self {
            sym: ident.to_string(),
            span: SerializedSpan::from_native(ident.span()),
        }
    }

    pub fn from_fallback(ident: Ident) -> Self {
        Self {
            sym: ident.to_string(),
            span: SerializedSpan::from_fallback(ident.span()),
        }
    }

    pub fn into_native(self) -> NativeIdent {
        NativeIdent::new(&self.sym, self.span.into_native())
    }

    pub fn into_fallback(self) -> Ident {
        Ident::new(&self.sym, self.span.into_fallback())
    }
}

impl SerializedPunct {
    pub fn from_native(punct: NativePunct) -> Self {
        Self {
            ch: punct.as_char(),
            spacing: SerializedSpacing::from_native(punct.spacing()),
            span: SerializedSpan::from_native(punct.span()),
        }
    }

    pub fn from_fallback(punct: Punct) -> Self {
        Self {
            ch: punct.as_char(),
            spacing: SerializedSpacing::from_fallback(punct.spacing()),
            span: SerializedSpan::from_fallback(punct.span()),
        }
    }

    pub fn into_native(self) -> NativePunct {
        let mut punct = NativePunct::new(self.ch, self.spacing.into_native());
        punct.set_span(self.span.into_native());
        punct
    }

    pub fn into_fallback(self) -> Punct {
        let mut punct = Punct::new(self.ch, self.spacing.into_fallback());
        punct.set_span(self.span.into_fallback());
        punct
    }
}

impl SerializedLiteral {
    pub fn from_native(literal: NativeLiteral) -> Self {
        Self {
            repr: literal.to_string(),
            span: SerializedSpan::from_native(literal.span()),
        }
    }

    pub fn from_fallback(literal: Literal) -> Self {
        Self {
            repr: literal.to_string(),
            span: SerializedSpan::from_fallback(literal.span()),
        }
    }

    pub fn into_native(self) -> NativeLiteral {
        let mut literal = NativeLiteral::from_str(&self.repr).unwrap();
        literal.set_span(self.span.into_native());
        literal
    }

    pub fn into_fallback(self) -> Literal {
        let mut literal = Literal::from_str(&self.repr).unwrap();
        literal.set_span(self.span.into_fallback());
        literal
    }
}

impl SerializedDelimiter {
    pub fn from_native(delimiter: NativeDelimiter) -> Self {
        match delimiter {
            NativeDelimiter::Parenthesis => Self::Parenthesis,
            NativeDelimiter::Brace => Self::Brace,
            NativeDelimiter::Bracket => Self::Bracket,
            NativeDelimiter::None => Self::None,
        }
    }

    pub fn from_fallback(delimiter: Delimiter) -> Self {
        match delimiter {
            Delimiter::Parenthesis => Self::Parenthesis,
            Delimiter::Brace => Self::Brace,
            Delimiter::Bracket => Self::Bracket,
            Delimiter::None => Self::None,
        }
    }

    pub fn into_native(self) -> NativeDelimiter {
        match self {
            Self::Parenthesis => NativeDelimiter::Parenthesis,
            Self::Brace => NativeDelimiter::Brace,
            Self::Bracket => NativeDelimiter::Bracket,
            Self::None => NativeDelimiter::None,
        }
    }

    pub fn into_fallback(self) -> Delimiter {
        match self {
            Self::Parenthesis => Delimiter::Parenthesis,
            Self::Brace => Delimiter::Brace,
            Self::Bracket => Delimiter::Bracket,
            Self::None => Delimiter::None,
        }
    }
}

impl SerializedSpacing {
    pub fn from_native(spacing: NativeSpacing) -> Self {
        match spacing {
            NativeSpacing::Alone => Self::Alone,
            NativeSpacing::Joint => Self::Joint,
        }
    }

    pub fn from_fallback(spacing: Spacing) -> Self {
        match spacing {
            Spacing::Alone => Self::Alone,
            Spacing::Joint => Self::Joint,
        }
    }

    pub fn into_native(self) -> NativeSpacing {
        match self {
            Self::Alone => NativeSpacing::Alone,
            Self::Joint => NativeSpacing::Joint,
        }
    }

    pub fn into_fallback(self) -> Spacing {
        match self {
            Self::Alone => Spacing::Alone,
            Self::Joint => Spacing::Joint,
        }
    }
}

impl SerializedSpan {
    pub fn from_native(_span: NativeSpan) -> Self {
        Self {}
    }

    pub fn from_fallback(_span: Span) -> Self {
        Self {}
    }

    pub fn into_native(self) -> NativeSpan {
        NativeSpan::call_site()
    }

    pub fn into_fallback(self) -> Span {
        Span::call_site()
    }
}

// === Surrogate Thread === //

type SurrogateInput<O> = Box<dyn Send + FnOnce() -> O>;
type SurrogateOutput<O> = Result<O, Box<dyn Send + Any>>;

pub struct SurrogateThread<O> {
    in_sender: mpsc::SyncSender<SurrogateInput<O>>,
    out_receiver: mpsc::Receiver<SurrogateOutput<O>>,
}

impl<O: 'static + Send> SurrogateThread<O> {
    pub fn new() -> Self {
        let (in_sender, in_receiver) = mpsc::sync_channel::<SurrogateInput<O>>(1);
        let (out_sender, out_receiver) = mpsc::sync_channel::<SurrogateOutput<O>>(1);

        std::thread::spawn(move || {
            while let Ok(msg) = in_receiver.recv() {
                let _ = out_sender.send(panic::catch_unwind(panic::AssertUnwindSafe(msg)));
            }
        });

        Self {
            in_sender,
            out_receiver,
        }
    }

    pub fn fire(&self, func: impl 'static + Send + FnOnce() -> O) -> O {
        self.in_sender.send(Box::new(func)).unwrap();
        self.out_receiver.recv().unwrap().unwrap()
    }
}

// === Smuggling Routines === //

// Native-backed proc-macro objects cannot persist between invocations of a proc-macro. This hack
// serves to "fix" that problem.
pub fn smuggle<const N: usize>(
    inputs: [NativeTokenStream; N],
    f: impl 'static + Send + FnOnce([TokenStream; N]) -> TokenStream,
) -> NativeTokenStream {
    // Serialize the input
    let input = inputs.map(SerializedTokenStream::from_native);

    // Pass it to the worker, who deserializes it into the dispatch-safe fallback format, processes
    // it, and returns its output in a serialized form.
    thread_local! {
        static SURROGATE: SurrogateThread<SerializedTokenStream> = SurrogateThread::new();
    }

    let output = SURROGATE.with(move |v| {
        v.fire(move || {
            SerializedTokenStream::from_fallback(f(input.map(SerializedTokenStream::into_fallback)))
        })
    });

    // Finally, convert that serialized output into the native form.
    output.into_native()
}
