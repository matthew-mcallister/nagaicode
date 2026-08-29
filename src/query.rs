use std::collections::HashMap;
use std::hash::BuildHasher;

use chrono::NaiveDateTime;
use serde_json::{Value, json};

use crate::arena::Id;
use crate::ui::style::Color;

/// An error encountered while querying the application.
#[derive(Debug, Clone)]
pub enum QueryError {
    InvalidField(String),
    DataError(String),
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::InvalidField(name) => write!(f, "invalid field name: {name}"),
            QueryError::DataError(msg) => write!(f, "data error: {msg}"),
        }
    }
}

/// Helper type for working with paths.
pub enum QueryField<'a> {
    Value(Value),
    DataQuery(&'a dyn DataQuery),
    Boxed(Box<dyn DataQuery + 'a>),
}

/// Helper method for working with paths. Returns `(head, tail)`, where head
/// is the first component in the path and `tail` is the rest of the path, if
/// any. Automatically strips leading/trailing slashes. When `head` is an empty
/// string, this means the full object is being queried.
pub fn split_path(path: &str) -> (&str, Option<&str>) {
    let path = path.trim_matches('/');
    match path.split_once('/') {
        Some((head, tail)) => (head, Some(tail)),
        None => (path, None),
    }
}

/// Debugging/testing trait for retrieving deeply nested application state in a
/// simple, uniform way. Conceptually, the entire application state may be
/// imagined as a huge JSON document. We can name a piece of data from this
/// document by specifying a URI or URL to retrieve.
///
/// URL syntax:
/// ```txt
///   [query://]/path/to/object/field
/// ```
///
/// To retrieve all fields of an object:
/// ```txt
///   [query://]/path/to/object
/// ```
///
/// N.B. Don't confuse this trait with diesel's Queryable trait. This trait is
/// not directly related to database queries.
pub trait DataQuery {
    /// Helper method that makes it easier to work with paths. This is the
    /// minimum required to implement DataQuery. When the query is an empty
    /// string, the whole object should be returned as a value.
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError>;

    /// Queries data by URI. Currently only `query://<path>` is supported.
    fn query(&self, uri: &str) -> Result<Value, QueryError> {
        let path = uri.strip_prefix("query://").unwrap_or(uri);
        let (head, tail) = split_path(path);
        let field = self.query_field(head)?;
        match field {
            QueryField::Value(value) => Ok(value),
            QueryField::DataQuery(child) => child.query(tail.unwrap_or("")),
            QueryField::Boxed(field) => field.query(tail.unwrap_or("")),
        }
    }
}

/// Exposed fields:
/// - `/<index>`: T, element at the given index
///
/// Querying the whole object returns an array of the elements.
impl<T> DataQuery for &[T]
    where T: DataQuery
{
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        if field.is_empty() {
            let arr: Vec<Value> = self
                .iter()
                .map(|t| t.query("/"))
                .collect::<Result<_, _>>()?;
            Ok(QueryField::Value(arr.into()))
        } else {
            let index: usize = field
                .parse()
                .map_err(|_| QueryError::InvalidField(field.to_string()))?;
            let item = self
                .get(index)
                .ok_or_else(|| QueryError::InvalidField(field.to_string()))?;
            Ok(QueryField::DataQuery(item))
        }
    }
}

impl<V: DataQuery, S: BuildHasher> DataQuery for HashMap<String, V, S> {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => {
                let mut map = serde_json::Map::new();
                for (k, v) in self.into_iter() {
                    map.insert(k.clone(), v.query("")?);
                }
                Ok(QueryField::Value(Value::Object(map)))
            },
            key => {
                let v = self.get(key).ok_or_else(|| QueryError::InvalidField(field.to_string()))?;
                Ok(QueryField::DataQuery(v))
            }
        }
    }
}

/// Trait for converting types to JSON. Mainly intended for primitive values
/// like strings and ints.
// TODO: Replace nontrivial uses of ToJson with DataQuery
pub trait ToJson {
    fn to_json(self) -> Value;
}

macro_rules! impl_to_json_primitive {
    ($($ty:ident $(<$($gen:ident)+>)?),*$(,)?) => {
        $(
            impl$(<$($gen)+: Into<Value>>)? ToJson for $ty $(<$($gen)+>)? {
                fn to_json(self) -> Value {
                    self.into()
                }
            }
        )*
    };
}

impl_to_json_primitive! {
    bool,
    i8, i16, i32, i64, isize,
    u8, u16, u32, u64, usize,
    f32, f64,
    String,
    Vec<T>,
}

impl<T: ToJson> ToJson for Option<T> {
    fn to_json(self) -> Value {
        match self {
            Some(value) => value.to_json(),
            None => Value::Null,
        }
    }
}

impl<T> ToJson for Id<T> {
    fn to_json(self) -> Value {
        json!([self.generation(), self.index()])
    }
}

impl ToJson for NaiveDateTime {
    fn to_json(self) -> Value {
        json!(self.format("%Y-%m-%dT%H:%M:%S%.f").to_string())
    }
}

impl ToJson for Color {
    fn to_json(self) -> Value {
        match self {
            Color::Black => json!("black"),
            Color::DarkGrey => json!("dark_grey"),
            Color::Red => json!("red"),
            Color::DarkRed => json!("dark_red"),
            Color::Green => json!("green"),
            Color::DarkGreen => json!("dark_green"),
            Color::Yellow => json!("yellow"),
            Color::DarkYellow => json!("dark_yellow"),
            Color::Blue => json!("blue"),
            Color::DarkBlue => json!("dark_blue"),
            Color::Magenta => json!("magenta"),
            Color::DarkMagenta => json!("dark_magenta"),
            Color::Cyan => json!("cyan"),
            Color::DarkCyan => json!("dark_cyan"),
            Color::White => json!("white"),
            Color::Grey => json!("grey"),
            Color::Rgb { r, g, b } => json!([r, g, b]),
            Color::AnsiValue(v) => v.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::markdown::ResumePoint;

    #[test]
    fn test_slice_query() {
        let data = [
            ResumePoint { offset: 1, row: 2 },
            ResumePoint { offset: 3, row: 4 },
        ];
        let slice: &[ResumePoint] = &data;
        let expected = json!([slice[0].query("/").unwrap(), slice[1].query("/").unwrap()]);
        assert_eq!(slice.query("/").unwrap(), expected);
        assert_eq!(slice.query("/0").unwrap(), slice[0].query("/").unwrap());
        assert_eq!(slice.query("/1").unwrap(), slice[1].query("/").unwrap());
    }

    #[test]
    fn test_datetime_to_json() {
        let dt = chrono::NaiveDate::from_ymd_opt(2015, 9, 18).unwrap().and_hms_opt(23, 56, 4).unwrap();
        assert_eq!(dt.to_json(), json!("2015-09-18T23:56:04"));

        let with_millis = chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap().and_hms_milli_opt(3, 4, 5, 600).unwrap();
        assert_eq!(with_millis.to_json(), json!("2024-01-02T03:04:05.600"));
    }

    #[test]
    fn test_primitive_to_json() {
        assert_eq!(true.to_json(), json!(true));
        assert_eq!(false.to_json(), json!(false));
        assert_eq!(42i32.to_json(), json!(42));
        assert_eq!(7u8.to_json(), json!(7));
        assert_eq!(1.5f32.to_json(), json!(1.5));
        assert_eq!("hello".to_owned().to_json(), json!("hello"));
        assert_eq!(vec![1, 2, 3].to_json(), json!([1, 2, 3]));
        assert_eq!(Some("x".to_owned()).to_json(), json!("x"));
        assert_eq!(None::<i32>.to_json(), json!(null));
    }
}
