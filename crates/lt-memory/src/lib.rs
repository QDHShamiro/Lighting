mod lessons;
mod project;
mod start;
mod store;
mod text;
mod vault;
mod wiki;

pub use lessons::Lesson;
pub use project::project_of;
pub use start::{Counts, Session};
pub use store::{Hit, NewNote, Note, Store, Todo, VEC_DIM, date, day, now};
pub use text::{fts_query, oneline, scrub, short, similar, stop_word};
pub use vault::{Page, claude_dirs, parse as parse_page};
pub use wiki::{Draft, accept as accept_draft, drafts, drop_draft, rel_path};
pub type DbResult<T> = rusqlite::Result<T>;
