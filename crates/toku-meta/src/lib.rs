mod error;
mod openlibrary;

pub use error::MetaError;
pub use openlibrary::{OpenLibraryBook, fetch_by_isbn, fetch_cover};
