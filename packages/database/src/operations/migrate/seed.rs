//! SQL seed files: every `*.sql` in a seed folder, applied in byte-wise filename order.
//!
//! Seed data is for local and shared development databases, never production. Schema belongs in
//! migrations, not in seed files.

use std::path::{Path, PathBuf};

use sqlx::AssertSqlSafe;

use crate::types::DbPool;

/// Seed folders shipped with this crate (`seed/local`, `seed/dev`).
pub const SEED_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/seed");

#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error("database.seed.read.failed {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("database.seed.query.failed {path}")]
    Query {
        path: PathBuf,
        #[source]
        source: sqlx::Error,
    },
    #[error("database.seed.skip-query.failed")]
    SkipQuery(#[source] sqlx::Error),
}

/// The `*.sql` files in `folder`, sorted byte-wise by name. Empty when the folder is missing.
pub fn files(folder: &Path) -> Result<Vec<PathBuf>, SeedError> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(SeedError::Read {
                path: folder.to_path_buf(),
                source,
            });
        }
    };

    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|source| SeedError::Read {
                path: folder.to_path_buf(),
                source,
            })?
            .path();
        if path.extension().is_some_and(|extension| extension == "sql") {
            paths.push(path);
        }
    }
    paths.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    Ok(paths)
}

/// `true` when `query` (a SQL expression returning one boolean) says seed should be skipped.
pub async fn populated(pool: &DbPool, query: &str) -> Result<bool, SeedError> {
    sqlx::query_scalar::<_, bool>(AssertSqlSafe(query.to_owned()))
        .fetch_one(pool)
        .await
        .map_err(SeedError::SkipQuery)
}

/// Applies each file in order and returns the paths applied.
pub async fn apply(pool: &DbPool, paths: &[PathBuf]) -> Result<Vec<PathBuf>, SeedError> {
    for path in paths {
        let sql = std::fs::read_to_string(path).map_err(|source| SeedError::Read {
            path: path.clone(),
            source,
        })?;
        sqlx::raw_sql(AssertSqlSafe(sql))
            .execute(pool)
            .await
            .map_err(|source| SeedError::Query {
                path: path.clone(),
                source,
            })?;
        tracing::info!(path = %path.display(), "database.seed.applied");
    }
    Ok(paths.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_shipped_local_seed_files() {
        let paths = files(&Path::new(SEED_ROOT).join("local")).expect("readable");
        assert!(
            paths
                .iter()
                .all(|path| path.extension().is_some_and(|ext| ext == "sql"))
        );
        assert!(!paths.is_empty());
    }

    #[test]
    fn treats_a_missing_folder_as_empty() {
        let paths = files(Path::new("/nonexistent/seed/folder")).expect("missing is fine");
        assert!(paths.is_empty());
    }
}
