//! grave-core: ヘッドレスのゲームロジック。
//!
//! 入力は「コロンコマンド」（`move west` など）の文字列だけ。
//! キーバインドや描画はこのクレートの外（tui）に置く。

pub mod command;
pub mod game;
pub mod item;
pub mod journal;
pub mod map;
pub mod monster;
pub mod record;
pub mod rng;
pub mod status;
pub mod trap;

pub use command::{Command, Dir, TravelTarget, ZapTarget, COMMAND_HELP, COMMAND_NAMES};
pub use game::{Cell, EnemyView, Game, ItemEntry, LogEntry, Outcome};
pub use item::{Class, ItemKind};
pub use status::{Change, Status, StatusEvent};
