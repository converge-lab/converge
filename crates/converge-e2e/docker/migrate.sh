export DATABASE_URL="postgres://$POSTGRES_USER@%2Fvar%2Frun%2Fpostgresql/$POSTGRES_DB"
sqlx migrate run --source /migrations
