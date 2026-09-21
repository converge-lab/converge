//! Separate database diagnostics from safe errors returned by the storage API.

use converge_storage::StoreError;

pub(crate) fn db_err(error: sqlx::Error) -> StoreError {
    if matches!(error, sqlx::Error::RowNotFound) {
        return StoreError::NotFound;
    }
    // PostgreSQL messages, details, identifiers and nested causes may contain
    // user values. Only the standardized SQLSTATE and a fixed category enter
    // diagnostics; never format the raw error, SQL, connection URL or binds.
    let code = error.as_database_error().and_then(|db| db.code());
    let code = code.as_deref().filter(|code| {
        code.len() == 5
            && code
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    });
    let (category, outcome) = match &error {
        sqlx::Error::Database(_) if code == Some("23503") => (
            "foreign_key_violation",
            StoreError::Invalid(
                "A referenced item no longer exists. Refresh the page and try again.".into(),
            ),
        ),
        sqlx::Error::Database(_) if code == Some("23505") => (
            "unique_violation",
            StoreError::Conflict(
                "This operation conflicts with an existing item. Refresh the page and try again."
                    .into(),
            ),
        ),
        sqlx::Error::Io(_) => (
            "connection_io",
            StoreError::Unavailable("database connection failed".into()),
        ),
        sqlx::Error::PoolTimedOut => (
            "pool_timeout",
            StoreError::Unavailable("database connection timed out".into()),
        ),
        sqlx::Error::PoolClosed => (
            "pool_closed",
            StoreError::Unavailable("database connection is closed".into()),
        ),
        sqlx::Error::Database(_) => (
            "database",
            StoreError::Backend("database operation failed".into()),
        ),
        sqlx::Error::Decode(_) | sqlx::Error::ColumnDecode { .. } => (
            "decode",
            StoreError::Backend("database response could not be read".into()),
        ),
        _ => (
            "driver",
            StoreError::Backend("database operation failed".into()),
        ),
    };
    tracing::error!(category, sqlstate = code, "database operation failed");
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use testcontainers_modules::{
        postgres::Postgres,
        testcontainers::{ImageExt, runners::AsyncRunner},
    };

    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn map_with_diagnostics(error: sqlx::Error) -> (StoreError, String) {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let capture = Capture(bytes.clone());
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || capture.clone())
            .finish();
        let mapped = tracing::subscriber::with_default(subscriber, || db_err(error));
        let logs = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        (mapped, logs)
    }

    #[tokio::test]
    async fn constraint_failures_keep_sqlstate_but_never_expose_database_text() {
        let pg = Postgres::default()
            .with_tag("16-alpine")
            .start()
            .await
            .unwrap();
        let port = pg.get_host_port_ipv4(5432).await.unwrap();
        let pool = sqlx::PgPool::connect(&format!(
            "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
        ))
        .await
        .unwrap();
        sqlx::raw_sql(
            r#"
            create table parent (id int primary key);
            create table child (
                id int, parent_id int,
                constraint "private-fk@example.test" foreign key (parent_id) references parent(id),
                constraint "private-unique@example.test" unique (id)
            );
            insert into parent values (1);
            insert into child values (1, 1);
        "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        for (query, code, expected) in [
            ("insert into child values (2, 2)", "23503", "invalid"),
            ("insert into child values (1, 1)", "23505", "conflict"),
        ] {
            let error = sqlx::query(query).execute(&pool).await.unwrap_err();
            assert!(error.to_string().contains("@example.test"));
            let (mapped, logs) = map_with_diagnostics(error);
            assert!(matches!(
                (&mapped, expected),
                (StoreError::Invalid(_), "invalid") | (StoreError::Conflict(_), "conflict")
            ));
            assert!(logs.contains(code), "SQLSTATE is missing: {logs}");
            for forbidden in ["@example.test", "child", "insert into", "postgres://"] {
                assert!(!mapped.to_string().contains(forbidden));
                assert!(!logs.contains(forbidden));
            }
        }
    }

    #[test]
    fn connection_failures_do_not_log_addresses_or_credentials() {
        let error = sqlx::Error::Io(std::io::Error::other(
            "postgres://user:secret@host/private@example.test",
        ));
        let (mapped, logs) = map_with_diagnostics(error);
        assert!(matches!(mapped, StoreError::Unavailable(_)));
        assert!(logs.contains("connection_io"));
        for forbidden in ["secret", "@host", "@example.test", "postgres://"] {
            assert!(!mapped.to_string().contains(forbidden));
            assert!(!logs.contains(forbidden));
        }
    }
}
