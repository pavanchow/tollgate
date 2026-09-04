use std::fmt;

#[derive(Debug)]
pub enum ConfigError {
    Parse { line: usize, message: String },
    Missing { block: usize, field: &'static str },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Parse { line, message } => {
                write!(f, "config parse error at line {line}: {message}")
            }
            ConfigError::Missing { block, field } => {
                write!(f, "route block #{block} is missing required field '{field}'")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug)]
pub enum RequestError {
    Empty,
    TooLarge,
    BadRequestLine,
    BadHeader,
    UnsupportedVersion,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            RequestError::Empty => "empty request",
            RequestError::TooLarge => "request head exceeded byte limit",
            RequestError::BadRequestLine => "malformed request line",
            RequestError::BadHeader => "malformed header line",
            RequestError::UnsupportedVersion => "unsupported HTTP version",
        };
        write!(f, "{s}")
    }
}

impl std::error::Error for RequestError {}
