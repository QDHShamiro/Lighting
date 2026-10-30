mod project;
mod start;
mod store;
mod text;

pub use project::project_of;
pub use start::Session;
pub use store::{Hit, NewNote, Note, Store, Todo, VEC_DIM, date, day, now};
pub use text::{fts_query, oneline, scrub, short, similar, stop_word};
pub type DbResult<T> = rusqlite::Result<T>;
