use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("file i/o failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("file {0} is empty")]
    EmptyFile(PathBuf),

    #[error("could not read {path} as UTF-8 text")]
    Encoding { path: PathBuf },

    #[error("{0}")]
    Invalid(String),
}

impl Error {
    /// Attaches the offending path to an I/O error. `std::io::Error` carries no path,
    /// and "No such file or directory" without a filename is useless in a log.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// Fills in the path on an error that came from `?` on a bare `std::io::Result`.
    pub fn with_path(self, path: impl Into<PathBuf>) -> Self {
        match self {
            Error::Io { source, .. } => Error::Io {
                path: path.into(),
                source,
            },
            other => other,
        }
    }
}

/// Lets `?` convert a bare `std::io::Error`. The path is empty until a caller supplies
/// it via [`Error::with_path`], which is why call sites that know the path should do so.
impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Error::Io {
            path: PathBuf::new(),
            source,
        }
    }
}
