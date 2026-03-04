use std::collections::HashMap;
use std::fmt;

/// Dynamic value type for adapter I/O and shared state.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    I64(i64),
    F64(f64),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Map(HashMap<String, Value>),
}

impl Value {
    pub fn str(s: &str) -> Self {
        Value::Str(s.to_string())
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::I64(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::F64(n) => Some(*n),
            Value::I64(n) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Value::None)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::None => write!(f, "None"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::I64(n) => write!(f, "{n}"),
            Value::F64(n) => write!(f, "{n}"),
            Value::Str(s) => write!(f, "{s}"),
            Value::Bytes(b) => write!(f, "<{} bytes>", b.len()),
            Value::List(l) => write!(f, "[{} items]", l.len()),
            Value::Map(m) => write!(f, "{{{} entries}}", m.len()),
        }
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self { Value::Bool(b) }
}
impl From<i64> for Value {
    fn from(n: i64) -> Self { Value::I64(n) }
}
impl From<f64> for Value {
    fn from(n: f64) -> Self { Value::F64(n) }
}
impl From<String> for Value {
    fn from(s: String) -> Self { Value::Str(s) }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self { Value::Str(s.to_string()) }
}
