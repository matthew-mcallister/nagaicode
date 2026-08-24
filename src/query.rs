use serde_json::{Value, json};

use crate::arena::Id;
use crate::ui::style::Color;

/// An error encountered while querying the application.
#[derive(Debug, Clone)]
pub enum QueryError {
    InvalidField(String),
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::InvalidField(name) => write!(f, "invalid field name: {name}"),
        }
    }
}

/// Helper type for working with paths.
pub enum QueryField<'a> {
    Value(Value),
    DataQuery(&'a dyn DataQuery)
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

/// Trait for converting types to JSON. Mainly intended for primitive values
/// like strings and ints.
pub trait ToJson {
    fn to_json(self) -> Value;
}

impl<T> ToJson for T
    where Value: From<T>
{
    fn to_json(self) -> Value {
        Value::from(self)
    }
}

impl<T> ToJson for Id<T> {
    fn to_json(self) -> Value {
        json!([self.generation(), self.index()])
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
}
