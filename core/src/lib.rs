//! colonrogue-core: ヘッドレスのゲームロジック。
//!
//! 入力は「コロンコマンド」（`move west` など）の文字列だけ。
//! キーバインドや描画はこのクレートの外（tui）に置く。

pub mod command;
pub mod game;
pub mod map;
pub mod record;
pub mod rng;

pub use command::{Command, Dir, TravelTarget, COMMAND_HELP, COMMAND_NAMES};
pub use game::{Cell, Game, LogEntry, Outcome};
