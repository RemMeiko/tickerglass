// 配置模块。分层：`schema`（结构定义）→ `migrate`（纯函数迁移）→ `store`（唯一 I/O）。
//
// `schema` 与 `migrate` 不碰文件系统，故可被单测直接调用且无需临时目录。

pub mod migrate;
pub mod schema;
pub mod store;

pub use migrate::{migrate, LoadOutcome, MigrationResult};
pub use schema::{
    AlertConfig, AppConfig, AppearanceConfig, RefreshConfig, Theme, WindowConfig,
    MINUTE_SECS_MAX, MINUTE_SECS_MIN, QUOTE_SECS_MAX, QUOTE_SECS_MIN, SCHEMA_VERSION,
};
