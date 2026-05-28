mod error;
mod openlibrary;

pub use error::MetaError;
pub use openlibrary::{OpenLibraryBook, SearchResult, fetch_by_isbn, fetch_cover, search_books};
