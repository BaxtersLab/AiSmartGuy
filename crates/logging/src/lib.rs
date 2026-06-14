pub mod errors;
pub mod types;
pub mod formatter;
pub mod file_writer;
pub mod metrics;
pub mod logger;

pub use errors::LogError;
pub use types::{LogEntry, LogLevel, Logger};
pub use logger::create_logger;

/// log_info!(logger, "message")
/// log_info!(logger, "message", { "key" => "val", ... })
#[macro_export]
macro_rules! log_info {
    ($logger:expr, $msg:expr) => {
        let _ = $logger.log($crate::LogLevel::Info, $msg, None);
    };
    ($logger:expr, $msg:expr, { $($k:expr => $v:expr),* }) => {{
        let mut _map = std::collections::HashMap::new();
        $( _map.insert($k.to_string(), $v.to_string()); )*
        let _ = $logger.log($crate::LogLevel::Info, $msg, Some(_map));
    }};
}

#[macro_export]
macro_rules! log_warn {
    ($logger:expr, $msg:expr) => {
        let _ = $logger.log($crate::LogLevel::Warning, $msg, None);
    };
    ($logger:expr, $msg:expr, { $($k:expr => $v:expr),* }) => {{
        let mut _map = std::collections::HashMap::new();
        $( _map.insert($k.to_string(), $v.to_string()); )*
        let _ = $logger.log($crate::LogLevel::Warning, $msg, Some(_map));
    }};
}

#[macro_export]
macro_rules! log_error {
    ($logger:expr, $msg:expr) => {
        let _ = $logger.log($crate::LogLevel::Error, $msg, None);
    };
    ($logger:expr, $msg:expr, { $($k:expr => $v:expr),* }) => {{
        let mut _map = std::collections::HashMap::new();
        $( _map.insert($k.to_string(), $v.to_string()); )*
        let _ = $logger.log($crate::LogLevel::Error, $msg, Some(_map));
    }};
}

#[macro_export]
macro_rules! log_debug {
    ($logger:expr, $msg:expr) => {
        let _ = $logger.log($crate::LogLevel::Debug, $msg, None);
    };
    ($logger:expr, $msg:expr, { $($k:expr => $v:expr),* }) => {{
        let mut _map = std::collections::HashMap::new();
        $( _map.insert($k.to_string(), $v.to_string()); )*
        let _ = $logger.log($crate::LogLevel::Debug, $msg, Some(_map));
    }};
}
