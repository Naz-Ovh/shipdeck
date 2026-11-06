//! The console of `apps/cli`: `deploy`, `rollback` and `releases` print what
//! the engine reports, answer its questions from the flags, and pick an exit
//! code. There is no screen and no keyboard here.

mod lines;
mod paint;
mod run;
mod table;

pub(crate) use run::{Choices, Console};
