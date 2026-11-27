//! The server's database (native only: the web game never has one): accounts, the instance
//! lock and what players own (their car, for now). SQLite (a file: playing alone, or a
//! server by default), MySQL/MariaDB or PostgreSQL, by the URL's scheme, through sqlx's
//! `Any` driver; each has its own migrations (`MIGRATIONS`), run on opening.
//!
//! A server talks to it through `Database`: a thread of its own runs the queries one after
//! another (and the password hashing, which is slow on purpose), so the game's frames never
//! wait for it; answers to logins come back to be picked up (`Database::answers`). The
//! server's `user` commands use a `Store` directly.
//!
//! **Accounts** are a name (the name in the game, unique whatever the case) and a password,
//! kept as an argon2 hash. **The instance lock**: several server instances may share one
//! database, and a player plays on one at a time: logging in takes a row naming the
//! instance, refreshed while they play (`REFRESH`), taken over by another instance once
//! it's older than `STALE`, deleted when they leave. **Cars**: each account's cars as text
//! (`saved::write`), the one driven (`DRIVEN`) and those in the garage (`GARAGE`), written
//! while they play and when they leave. **Items**: loose parts (`saved::write_items`, one
//! each), in the garage or held at a shop (`held_at`). **Wallets**: each account's money,
//! changed only together with what it pays for (`pay`). **Offers** at the marketplace,
//! shared by every instance: what's offered (as text, gone from the seller's belongings
//! while it's up) and its price; buying one is one transaction that takes the offer only if
//! it's still there (`DELETE` counting the rows: of two buyers at once, one gets it), moves
//! the money and puts it in the buyer's garage.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use sqlx::AnyPool;
use sqlx::any::{AnyPoolOptions, AnyRow};
use sqlx::{AssertSqlSafe, Row};

/// An account's number in the database.
pub type AccountId = i64;

/// The account playing alone uses (made when first needed, without a password).
pub const DEFAULT_USER: &str = "default";

/// The longest name (characters).
pub const NAME_LENGTH: usize = 24;

/// How often a playing account's lock is refreshed, and how old one is when another
/// instance may take it over.
pub const REFRESH: Duration = Duration::from_secs(10);
pub const STALE: Duration = Duration::from_secs(30);

pub use crate::net::accounts::{
    Belongings, CAR, DRIVEN, GARAGE, MOST_OFFERS, MarketReply, OfferRow, PART, held_at,
};

/// The most offers listed at once (the newest).
const LISTED: i64 = 300;

/// What a login with a wrong name or password is told (the same for both).
const WRONG: &str = "Wrong name or password";

/// An account logged in: its number, its name as made, and what it owns as saved.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub belongings: Belongings,
    /// What's in its wallet (cents).
    pub money: i64,
}

/// Who came in.
#[derive(Clone, Debug, PartialEq)]
pub enum Login {
    Account(Account),
    /// A guest by this name: nothing of theirs is kept.
    Guest(String),
}

/// Who a server lets in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Admission {
    /// Accounts by their password, and (if allowed) guests: a name that isn't an account's,
    /// without a password.
    Accounts { guests: bool },
    /// Playing alone: whoever comes is the default user (`DEFAULT_USER`).
    Alone,
}

/// A name for an account or a guest: trimmed, 1 to `NAME_LENGTH` characters, no control
/// characters.
pub fn clean_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        Err("Give a name".into())
    } else if name.chars().count() > NAME_LENGTH {
        Err(format!("A name has at most {NAME_LENGTH} characters"))
    } else if name.chars().any(char::is_control) {
        Err("A name can't have control characters".into())
    } else {
        Ok(name.to_string())
    }
}

/// What two names are the same as, whatever the case.
fn folded(name: &str) -> String {
    name.to_lowercase()
}

/// Milliseconds since the Unix epoch.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// The SQL dialects there are.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Dialect {
    Sqlite,
    MySql,
    Postgres,
}

/// Each migration's statements for each dialect, in order (the version after the n-th is
/// n + 1).
struct Migration {
    sqlite: &'static [&'static str],
    mysql: &'static [&'static str],
    postgres: &'static [&'static str],
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        sqlite: &[
            "CREATE TABLE accounts (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, \
         folded TEXT NOT NULL UNIQUE, password TEXT NOT NULL, created BIGINT NOT NULL)",
            "CREATE TABLE active (account BIGINT PRIMARY KEY, instance TEXT NOT NULL, \
         refreshed BIGINT NOT NULL)",
            "CREATE TABLE cars (id INTEGER PRIMARY KEY AUTOINCREMENT, owner BIGINT NOT NULL, \
         data TEXT NOT NULL, saved BIGINT NOT NULL)",
            "CREATE INDEX cars_owner ON cars (owner)",
        ],
        mysql: &[
            "CREATE TABLE accounts (id BIGINT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(64) NOT \
         NULL, folded VARCHAR(64) NOT NULL UNIQUE, password VARCHAR(255) NOT NULL, created \
         BIGINT NOT NULL) CHARACTER SET utf8mb4",
            "CREATE TABLE active (account BIGINT PRIMARY KEY, instance VARCHAR(64) NOT NULL, \
         refreshed BIGINT NOT NULL) CHARACTER SET utf8mb4",
            "CREATE TABLE cars (id BIGINT AUTO_INCREMENT PRIMARY KEY, owner BIGINT NOT NULL, \
         data MEDIUMTEXT NOT NULL, saved BIGINT NOT NULL, INDEX cars_owner (owner)) \
         CHARACTER SET utf8mb4",
        ],
        postgres: &[
            "CREATE TABLE accounts (id BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, name \
         TEXT NOT NULL, folded TEXT NOT NULL UNIQUE, password TEXT NOT NULL, created BIGINT \
         NOT NULL)",
            "CREATE TABLE active (account BIGINT PRIMARY KEY, instance TEXT NOT NULL, refreshed \
         BIGINT NOT NULL)",
            "CREATE TABLE cars (id BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, owner \
         BIGINT NOT NULL, data TEXT NOT NULL, saved BIGINT NOT NULL)",
            "CREATE INDEX cars_owner ON cars (owner)",
        ],
    },
    // Several cars (the one driven, the garage's) and loose parts.
    Migration {
        sqlite: &[
            "ALTER TABLE cars ADD COLUMN place TEXT NOT NULL DEFAULT 'driven'",
            "CREATE TABLE items (id INTEGER PRIMARY KEY AUTOINCREMENT, owner BIGINT NOT NULL, \
         place TEXT NOT NULL, data TEXT NOT NULL, saved BIGINT NOT NULL)",
            "CREATE INDEX items_owner ON items (owner)",
        ],
        mysql: &[
            "ALTER TABLE cars ADD COLUMN place VARCHAR(160) NOT NULL DEFAULT 'driven'",
            "CREATE TABLE items (id BIGINT AUTO_INCREMENT PRIMARY KEY, owner BIGINT NOT NULL, \
         place VARCHAR(160) NOT NULL, data MEDIUMTEXT NOT NULL, saved BIGINT NOT NULL, \
         INDEX items_owner (owner)) CHARACTER SET utf8mb4",
        ],
        postgres: &[
            "ALTER TABLE cars ADD COLUMN place TEXT NOT NULL DEFAULT 'driven'",
            "CREATE TABLE items (id BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, owner \
         BIGINT NOT NULL, place TEXT NOT NULL, data TEXT NOT NULL, saved BIGINT NOT NULL)",
            "CREATE INDEX items_owner ON items (owner)",
        ],
    },
    // Every account's wallet (cents), new ones starting with `trade::START_MONEY` (a test
    // checks they agree).
    Migration {
        sqlite: &["ALTER TABLE accounts ADD COLUMN money BIGINT NOT NULL DEFAULT 500000"],
        mysql: &["ALTER TABLE accounts ADD COLUMN money BIGINT NOT NULL DEFAULT 500000"],
        postgres: &["ALTER TABLE accounts ADD COLUMN money BIGINT NOT NULL DEFAULT 500000"],
    },
    // The marketplace's offers.
    Migration {
        sqlite: &[
            "CREATE TABLE offers (id INTEGER PRIMARY KEY AUTOINCREMENT, seller BIGINT NOT NULL, \
         seller_name TEXT NOT NULL, what TEXT NOT NULL, data TEXT NOT NULL, price BIGINT NOT \
         NULL, made BIGINT NOT NULL)",
            "CREATE INDEX offers_seller ON offers (seller)",
        ],
        mysql: &[
            "CREATE TABLE offers (id BIGINT AUTO_INCREMENT PRIMARY KEY, seller BIGINT NOT NULL, \
         seller_name VARCHAR(64) NOT NULL, what VARCHAR(8) NOT NULL, data MEDIUMTEXT NOT NULL, \
         price BIGINT NOT NULL, made BIGINT NOT NULL, INDEX offers_seller (seller)) CHARACTER \
         SET utf8mb4",
        ],
        postgres: &[
            "CREATE TABLE offers (id BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, seller \
         BIGINT NOT NULL, seller_name TEXT NOT NULL, what TEXT NOT NULL, data TEXT NOT NULL, \
         price BIGINT NOT NULL, made BIGINT NOT NULL)",
            "CREATE INDEX offers_seller ON offers (seller)",
        ],
    },
];

/// A connection to the database, for running its queries (async: `Database` runs them on
/// a thread of its own, the `user` commands on a runtime of their own).
pub struct Store {
    pool: AnyPool,
    dialect: Dialect,
}

/// A database error as text.
fn text(e: sqlx::Error) -> String {
    e.to_string()
}

impl Store {
    /// Connects to the database at `url` and brings its tables up to date. A plain path
    /// is an SQLite file; SQLite files are made if they aren't there.
    pub async fn open(url: &str) -> Result<Self, String> {
        sqlx::any::install_default_drivers();
        let scheme = url.split_once(':').map_or("", |(scheme, _)| scheme);
        let (dialect, url) = match scheme {
            "sqlite" => (Dialect::Sqlite, url.to_string()),
            "mysql" | "mariadb" => (Dialect::MySql, url.replacen("mariadb:", "mysql:", 1)),
            "postgres" | "postgresql" => (Dialect::Postgres, url.to_string()),
            _ if !url.contains("://") => (Dialect::Sqlite, format!("sqlite://{url}")),
            other => return Err(format!("not a database URL scheme: {other}")),
        };
        let memory = url.contains(":memory:");
        // An SQLite file is made if it isn't there.
        let url = if dialect == Dialect::Sqlite && !memory && !url.contains("mode=") {
            let joint = if url.contains('?') { '&' } else { '?' };
            format!("{url}{joint}mode=rwc")
        } else {
            url
        };
        // SQLite writes one at a time anyway (and an in-memory one is one connection's).
        let options = match dialect {
            Dialect::Sqlite => AnyPoolOptions::new()
                .max_connections(1)
                .min_connections(1)
                .idle_timeout(None)
                .max_lifetime(None),
            _ => AnyPoolOptions::new().max_connections(4),
        };
        let pool = options.connect(&url).await.map_err(text)?;
        let store = Self { pool, dialect };
        store.migrate().await?;
        Ok(store)
    }

    /// `sql` with `?` placeholders in this dialect (PostgreSQL numbers them).
    fn sql(&self, sql: &'static str) -> AssertSqlSafe<String> {
        if self.dialect != Dialect::Postgres {
            return AssertSqlSafe(sql.to_string());
        }
        let mut out = String::with_capacity(sql.len() + 8);
        let mut n = 0;
        for c in sql.chars() {
            if c == '?' {
                n += 1;
                out += &format!("${n}");
            } else {
                out.push(c);
            }
        }
        AssertSqlSafe(out)
    }

    /// Runs the migrations the database hasn't had yet.
    async fn migrate(&self) -> Result<(), String> {
        sqlx::query("CREATE TABLE IF NOT EXISTS schema_version (version BIGINT NOT NULL)")
            .execute(&self.pool)
            .await
            .map_err(text)?;
        let version: Option<i64> = sqlx::query("SELECT MAX(version) FROM schema_version")
            .fetch_one(&self.pool)
            .await
            .map_err(text)?
            .try_get(0)
            .map_err(text)?;
        let version = version.unwrap_or(0).max(0) as usize;
        if version > MIGRATIONS.len() {
            return Err(format!(
                "the database is of a newer version of the game ({version}, this one knows \
                 up to {})",
                MIGRATIONS.len()
            ));
        }
        for (done, migration) in MIGRATIONS.iter().enumerate().skip(version) {
            let statements = match self.dialect {
                Dialect::Sqlite => migration.sqlite,
                Dialect::MySql => migration.mysql,
                Dialect::Postgres => migration.postgres,
            };
            let mut tx = self.pool.begin().await.map_err(text)?;
            for statement in statements {
                sqlx::query(*statement)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| format!("migration {}: {e}", done + 1))?;
            }
            sqlx::query(self.sql("INSERT INTO schema_version (version) VALUES (?)"))
                .bind(done as i64 + 1)
                .execute(&mut *tx)
                .await
                .map_err(text)?;
            tx.commit().await.map_err(text)?;
        }
        Ok(())
    }

    /// The account of `name` (whatever the case): its number, name and password hash.
    async fn account(&self, name: &str) -> Result<Option<(AccountId, String, String)>, String> {
        let row: Option<AnyRow> =
            sqlx::query(self.sql("SELECT id, name, password FROM accounts WHERE folded = ?"))
                .bind(folded(name))
                .fetch_optional(&self.pool)
                .await
                .map_err(text)?;
        row.map(|row| {
            Ok((
                row.try_get(0).map_err(text)?,
                row.try_get(1).map_err(text)?,
                row.try_get(2).map_err(text)?,
            ))
        })
        .transpose()
    }

    /// Makes an account. An empty password is none: nobody can log in to it (the default
    /// user's).
    pub async fn add_user(&self, name: &str, password: &str) -> Result<AccountId, String> {
        let name = clean_name(name)?;
        if self.account(&name).await?.is_some() {
            return Err(format!("There's already an account named {name}"));
        }
        let hash = hash(password)?;
        sqlx::query(
            self.sql("INSERT INTO accounts (name, folded, password, created) VALUES (?, ?, ?, ?)"),
        )
        .bind(&name)
        .bind(folded(&name))
        .bind(hash)
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .map_err(text)?;
        let (id, ..) = self.account(&name).await?.ok_or("the account went")?;
        Ok(id)
    }

    /// Gives an account a new password.
    pub async fn set_password(&self, name: &str, password: &str) -> Result<(), String> {
        let hash = hash(password)?;
        let done = sqlx::query(self.sql("UPDATE accounts SET password = ? WHERE folded = ?"))
            .bind(hash)
            .bind(folded(name))
            .execute(&self.pool)
            .await
            .map_err(text)?;
        if done.rows_affected() == 0 {
            return Err(format!("There's no account named {}", name.trim()));
        }
        Ok(())
    }

    /// Removes an account and everything it owns.
    pub async fn remove_user(&self, name: &str) -> Result<(), String> {
        let Some((id, ..)) = self.account(name).await? else {
            return Err(format!("There's no account named {}", name.trim()));
        };
        let mut tx = self.pool.begin().await.map_err(text)?;
        for sql in [
            "DELETE FROM cars WHERE owner = ?",
            "DELETE FROM items WHERE owner = ?",
            "DELETE FROM offers WHERE seller = ?",
            "DELETE FROM active WHERE account = ?",
            "DELETE FROM accounts WHERE id = ?",
        ] {
            sqlx::query(self.sql(sql))
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(text)?;
        }
        tx.commit().await.map_err(text)
    }

    /// Every account's name and when it was made (ms since the epoch), by name.
    pub async fn users(&self) -> Result<Vec<(String, i64)>, String> {
        let rows = sqlx::query("SELECT name, created FROM accounts ORDER BY folded")
            .fetch_all(&self.pool)
            .await
            .map_err(text)?;
        rows.iter()
            .map(|row| Ok((row.try_get(0).map_err(text)?, row.try_get(1).map_err(text)?)))
            .collect()
    }

    /// Lets someone in as `name` with `password` (empty: none), as `admission` allows,
    /// taking their account's lock for `instance` (by force playing alone: nobody else
    /// plays on that database).
    pub async fn login(
        &self,
        name: &str,
        password: &str,
        admission: Admission,
        instance: &str,
    ) -> Result<Login, String> {
        let (id, name) = match admission {
            Admission::Alone => match self.account(DEFAULT_USER).await? {
                Some((id, name, _)) => (id, name),
                None => (self.add_user(DEFAULT_USER, "").await?, DEFAULT_USER.into()),
            },
            Admission::Accounts { guests } => {
                let name = clean_name(name)?;
                let account = self.account(&name).await?;
                match account {
                    None if password.is_empty() && guests => return Ok(Login::Guest(name)),
                    None if password.is_empty() => {
                        return Err("This server lets in only players with an account: \
                                    give your account's name and password (its admin makes \
                                    accounts)"
                            .into());
                    }
                    Some(_) if password.is_empty() => {
                        return Err(format!(
                            "{name} is an account's name on this server: give its password"
                        ));
                    }
                    None => return Err(WRONG.into()),
                    Some((id, name, hash)) => {
                        if !verify(password, &hash) {
                            return Err(WRONG.into());
                        }
                        (id, name)
                    }
                }
            }
        };
        let force = admission == Admission::Alone;
        if !self.lock(id, instance, force).await? {
            return Err(
                "You're playing on another of this server's instances (if you just \
                        left it, try again in half a minute)"
                    .into(),
            );
        }
        let belongings = self.belongings(id).await?;
        let money = self.money(id).await?;
        Ok(Login::Account(Account {
            id,
            name,
            belongings,
            money,
        }))
    }

    /// Takes account `id`'s lock for `instance`: whether it got it (its own, a stale one,
    /// none there, or by `force`).
    async fn lock(&self, id: AccountId, instance: &str, force: bool) -> Result<bool, String> {
        let now = now_ms();
        let stale = now - STALE.as_millis() as i64;
        let taken = sqlx::query(self.sql(
            "UPDATE active SET instance = ?, refreshed = ? WHERE account = ? AND \
             (instance = ? OR refreshed < ? OR ?)",
        ))
        .bind(instance)
        .bind(now)
        .bind(id)
        .bind(instance)
        .bind(stale)
        .bind(force)
        .execute(&self.pool)
        .await
        .map_err(text)?;
        if taken.rows_affected() > 0 {
            return Ok(true);
        }
        let added = sqlx::query(
            self.sql("INSERT INTO active (account, instance, refreshed) VALUES (?, ?, ?)"),
        )
        .bind(id)
        .bind(instance)
        .bind(now)
        .execute(&self.pool)
        .await;
        if added.is_ok() {
            return Ok(true);
        }
        // There's a row: ours after all (MySQL counts only rows it changed), or another's.
        let holder: Option<AnyRow> =
            sqlx::query(self.sql("SELECT instance FROM active WHERE account = ?"))
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(text)?;
        let holder: Option<String> = holder.map(|row| row.try_get(0)).transpose().map_err(text)?;
        Ok(holder.as_deref() == Some(instance))
    }

    /// Refreshes the locks of the accounts playing on `instance`.
    pub async fn refresh(&self, instance: &str, accounts: &[AccountId]) -> Result<(), String> {
        let now = now_ms();
        for &id in accounts {
            sqlx::query(
                self.sql("UPDATE active SET refreshed = ? WHERE account = ? AND instance = ?"),
            )
            .bind(now)
            .bind(id)
            .bind(instance)
            .execute(&self.pool)
            .await
            .map_err(text)?;
        }
        Ok(())
    }

    /// Lets go of account `id`'s lock (if `instance` still has it).
    pub async fn release(&self, instance: &str, id: AccountId) -> Result<(), String> {
        sqlx::query(self.sql("DELETE FROM active WHERE account = ? AND instance = ?"))
            .bind(id)
            .bind(instance)
            .execute(&self.pool)
            .await
            .map_err(text)?;
        Ok(())
    }

    /// Account `id`'s car driven, as saved.
    pub async fn car(&self, id: AccountId) -> Result<Option<String>, String> {
        let row: Option<AnyRow> = sqlx::query(
            self.sql("SELECT data FROM cars WHERE owner = ? AND place = ? ORDER BY id LIMIT 1"),
        )
        .bind(id)
        .bind(DRIVEN)
        .fetch_optional(&self.pool)
        .await
        .map_err(text)?;
        row.map(|row| row.try_get(0)).transpose().map_err(text)
    }

    /// Everything account `id` owns, as saved.
    pub async fn belongings(&self, id: AccountId) -> Result<Belongings, String> {
        let car = self.car(id).await?;
        let rows = sqlx::query(
            self.sql("SELECT data FROM cars WHERE owner = ? AND place = ? ORDER BY id"),
        )
        .bind(id)
        .bind(GARAGE)
        .fetch_all(&self.pool)
        .await
        .map_err(text)?;
        let garage = (rows.iter())
            .map(|row| row.try_get(0).map_err(text))
            .collect::<Result<_, _>>()?;
        let rows =
            sqlx::query(self.sql("SELECT place, data FROM items WHERE owner = ? ORDER BY id"))
                .bind(id)
                .fetch_all(&self.pool)
                .await
                .map_err(text)?;
        let items = (rows.iter())
            .map(|row| Ok((row.try_get(0).map_err(text)?, row.try_get(1).map_err(text)?)))
            .collect::<Result<_, String>>()?;
        Ok(Belongings { car, garage, items })
    }

    /// Saves account `id`'s car driven.
    pub async fn save_car(&self, id: AccountId, car: &str) -> Result<(), String> {
        let now = now_ms();
        let updated = sqlx::query(
            self.sql("UPDATE cars SET data = ?, saved = ? WHERE owner = ? AND place = ?"),
        )
        .bind(car)
        .bind(now)
        .bind(id)
        .bind(DRIVEN)
        .execute(&self.pool)
        .await
        .map_err(text)?;
        if updated.rows_affected() == 0 && self.car(id).await?.is_none() {
            sqlx::query(
                self.sql("INSERT INTO cars (owner, data, saved, place) VALUES (?, ?, ?, ?)"),
            )
            .bind(id)
            .bind(car)
            .bind(now)
            .bind(DRIVEN)
            .execute(&self.pool)
            .await
            .map_err(text)?;
        }
        Ok(())
    }

    /// Saves everything account `id` owns, in place of what was saved.
    pub async fn save_belongings(
        &self,
        id: AccountId,
        belongings: &Belongings,
    ) -> Result<(), String> {
        let mut tx = self.pool.begin().await.map_err(text)?;
        self.put_belongings(&mut tx, id, belongings).await?;
        tx.commit().await.map_err(text)
    }

    /// What's in account `id`'s wallet (cents).
    pub async fn money(&self, id: AccountId) -> Result<i64, String> {
        let row = sqlx::query(self.sql("SELECT money FROM accounts WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(text)?;
        row.ok_or("no such account")?.try_get(0).map_err(text)
    }

    /// The wallets of the accounts `ids` (those there are).
    pub async fn balances(&self, ids: &[AccountId]) -> Result<Vec<(AccountId, i64)>, String> {
        let mut out = Vec::with_capacity(ids.len());
        for &id in ids {
            if let Ok(money) = self.money(id).await {
                out.push((id, money));
            }
        }
        Ok(out)
    }

    /// Adds `delta` cents to account `id`'s wallet (taking: never below nothing), and saves
    /// what it owns now (`belongings`, if given) with it, in one go: either both are done or
    /// neither.
    pub async fn pay(
        &self,
        id: AccountId,
        delta: i64,
        belongings: Option<&Belongings>,
    ) -> Result<(), String> {
        let mut tx = self.pool.begin().await.map_err(text)?;
        self.add_money(&mut tx, id, delta).await?;
        if let Some(belongings) = belongings {
            self.put_belongings(&mut tx, id, belongings).await?;
        }
        tx.commit().await.map_err(text)
    }

    /// Adds `delta` cents to account `id`'s wallet within `tx`; taking more than there is
    /// is refused.
    async fn add_money(
        &self,
        tx: &mut sqlx::Transaction<'static, sqlx::Any>,
        id: AccountId,
        delta: i64,
    ) -> Result<(), String> {
        if delta == 0 {
            return Ok(());
        }
        let done = sqlx::query(
            self.sql("UPDATE accounts SET money = money + ? WHERE id = ? AND money + ? >= 0"),
        )
        .bind(delta)
        .bind(id)
        .bind(delta)
        .execute(&mut **tx)
        .await
        .map_err(text)?;
        if done.rows_affected() == 0 {
            return Err("Not enough money".into());
        }
        Ok(())
    }

    /// Puts up an offer at the marketplace: `data` (a part's or car's text, `what` says
    /// which) for `price` cents, by account `seller` (called `seller_name`), whose belongings
    /// without it are `belongings`: saved in one go with the offer. Its number.
    pub async fn offer(
        &self,
        (seller, seller_name): (AccountId, &str),
        (what, data, price): (&str, &str, i64),
        belongings: &Belongings,
    ) -> Result<i64, String> {
        if what != PART && what != CAR {
            return Err(format!("not a part or a car: {what}"));
        }
        let mut tx = self.pool.begin().await.map_err(text)?;
        let count: i64 = sqlx::query(self.sql("SELECT COUNT(*) FROM offers WHERE seller = ?"))
            .bind(seller)
            .fetch_one(&mut *tx)
            .await
            .map_err(text)?
            .try_get(0)
            .map_err(text)?;
        if count >= MOST_OFFERS as i64 {
            return Err(format!("You have {MOST_OFFERS} offers up already"));
        }
        sqlx::query(self.sql(
            "INSERT INTO offers (seller, seller_name, what, data, price, made) VALUES (?, ?, ?, \
             ?, ?, ?)",
        ))
        .bind(seller)
        .bind(seller_name)
        .bind(what)
        .bind(data)
        .bind(price)
        .bind(now_ms())
        .execute(&mut *tx)
        .await
        .map_err(text)?;
        let id: Option<i64> = sqlx::query(self.sql("SELECT MAX(id) FROM offers WHERE seller = ?"))
            .bind(seller)
            .fetch_one(&mut *tx)
            .await
            .map_err(text)?
            .try_get(0)
            .map_err(text)?;
        self.put_belongings(&mut tx, seller, belongings).await?;
        tx.commit().await.map_err(text)?;
        id.ok_or_else(|| "the offer went".into())
    }

    /// The offers at the marketplace (the newest `LISTED`), oldest first.
    pub async fn offers(&self) -> Result<Vec<OfferRow>, String> {
        let rows = sqlx::query(self.sql(
            "SELECT id, seller, seller_name, what, data, price FROM offers ORDER BY id DESC \
             LIMIT ?",
        ))
        .bind(LISTED)
        .fetch_all(&self.pool)
        .await
        .map_err(text)?;
        let mut offers = rows.iter().map(offer_row).collect::<Result<Vec<_>, _>>()?;
        offers.reverse();
        Ok(offers)
    }

    /// The offer numbered `id`, within `tx`.
    async fn offer_in(
        &self,
        tx: &mut sqlx::Transaction<'static, sqlx::Any>,
        id: i64,
    ) -> Result<Option<OfferRow>, String> {
        let row: Option<AnyRow> = sqlx::query(
            self.sql("SELECT id, seller, seller_name, what, data, price FROM offers WHERE id = ?"),
        )
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(text)?;
        row.as_ref().map(offer_row).transpose()
    }

    /// Takes offer `id` off the marketplace, if it's still there: true if this took it (of
    /// several at once, only one does).
    async fn take_offer(
        &self,
        tx: &mut sqlx::Transaction<'static, sqlx::Any>,
        id: i64,
    ) -> Result<bool, String> {
        let done = sqlx::query(self.sql("DELETE FROM offers WHERE id = ?"))
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(text)?;
        Ok(done.rows_affected() == 1)
    }

    /// Puts `offer`'s part or car into account `owner`'s garage, within `tx`.
    async fn put_in_garage(
        &self,
        tx: &mut sqlx::Transaction<'static, sqlx::Any>,
        owner: AccountId,
        offer: &OfferRow,
    ) -> Result<(), String> {
        let sql = if offer.what == CAR {
            "INSERT INTO cars (owner, place, data, saved) VALUES (?, ?, ?, ?)"
        } else {
            "INSERT INTO items (owner, place, data, saved) VALUES (?, ?, ?, ?)"
        };
        sqlx::query(self.sql(sql))
            .bind(owner)
            .bind(GARAGE)
            .bind(&offer.data)
            .bind(now_ms())
            .execute(&mut **tx)
            .await
            .map_err(text)?;
        Ok(())
    }

    /// Account `seller` takes their offer `id` back: it goes into their garage. What it was.
    pub async fn withdraw(&self, seller: AccountId, id: i64) -> Result<OfferRow, String> {
        let mut tx = self.pool.begin().await.map_err(text)?;
        let offer = (self.offer_in(&mut tx, id).await?)
            .filter(|offer| offer.seller == seller)
            .ok_or("That offer isn't up any more")?;
        if !self.take_offer(&mut tx, id).await? {
            return Err("That offer isn't up any more".into());
        }
        self.put_in_garage(&mut tx, seller, &offer).await?;
        tx.commit().await.map_err(text)?;
        Ok(offer)
    }

    /// Account `buyer` (none: a guest, whose money and garage the server keeps) buys offer
    /// `id` for `price` cents (what they were shown: a changed price is refused): taken off
    /// the marketplace if it's still there, the money moved from the buyer to the seller, it
    /// into the buyer's garage, all in one go. What it was.
    pub async fn buy_offer(
        &self,
        buyer: Option<AccountId>,
        id: i64,
        price: i64,
    ) -> Result<OfferRow, String> {
        let mut tx = self.pool.begin().await.map_err(text)?;
        let offer = (self.offer_in(&mut tx, id).await?).ok_or("Someone bought that before you")?;
        if offer.price != price {
            return Err("Its price changed".into());
        }
        if buyer == Some(offer.seller) {
            return Err("That's your own offer".into());
        }
        if !self.take_offer(&mut tx, id).await? {
            return Err("Someone bought that before you".into());
        }
        if let Some(buyer) = buyer {
            self.add_money(&mut tx, buyer, -price).await?;
            self.put_in_garage(&mut tx, buyer, &offer).await?;
        }
        self.add_money(&mut tx, offer.seller, price).await?;
        tx.commit().await.map_err(text)?;
        Ok(offer)
    }

    /// Everything account `id` owns saved within `tx`, in place of what was saved.
    async fn put_belongings(
        &self,
        tx: &mut sqlx::Transaction<'static, sqlx::Any>,
        id: AccountId,
        belongings: &Belongings,
    ) -> Result<(), String> {
        let now = now_ms();
        for sql in [
            "DELETE FROM cars WHERE owner = ?",
            "DELETE FROM items WHERE owner = ?",
        ] {
            sqlx::query(self.sql(sql))
                .bind(id)
                .execute(&mut **tx)
                .await
                .map_err(text)?;
        }
        let cars = (belongings.car.iter().map(|car| (DRIVEN, car)))
            .chain(belongings.garage.iter().map(|car| (GARAGE, car)));
        for (place, car) in cars {
            sqlx::query(
                self.sql("INSERT INTO cars (owner, data, saved, place) VALUES (?, ?, ?, ?)"),
            )
            .bind(id)
            .bind(car)
            .bind(now)
            .bind(place)
            .execute(&mut **tx)
            .await
            .map_err(text)?;
        }
        for (place, item) in &belongings.items {
            sqlx::query(
                self.sql("INSERT INTO items (owner, place, data, saved) VALUES (?, ?, ?, ?)"),
            )
            .bind(id)
            .bind(place)
            .bind(item)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(text)?;
        }
        Ok(())
    }
}

/// An offer as it was read.
fn offer_row(row: &AnyRow) -> Result<OfferRow, String> {
    Ok(OfferRow {
        id: row.try_get(0).map_err(text)?,
        seller: row.try_get(1).map_err(text)?,
        seller_name: row.try_get(2).map_err(text)?,
        what: row.try_get(3).map_err(text)?,
        data: row.try_get(4).map_err(text)?,
        price: row.try_get(5).map_err(text)?,
    })
}

/// `password`'s argon2 hash (a PHC string); none for an empty password, which nothing
/// matches.
fn hash(password: &str) -> Result<String, String> {
    if password.is_empty() {
        return Ok(String::new());
    }
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|e| format!("couldn't hash the password: {e}"))
}

/// Whether `password` is the one `hash` was made of.
fn verify(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

/// What the database thread is asked to do.
enum Job {
    Login {
        peer: u32,
        name: String,
        password: String,
        admission: Admission,
    },
    Save {
        account: AccountId,
        car: String,
    },
    SaveAll {
        account: AccountId,
        belongings: Belongings,
    },
    /// Locks refreshed, and the wallets told (`Answer::Balances`): another instance's
    /// players may have paid into them.
    Refresh(Vec<AccountId>),
    Release(AccountId),
    Pay {
        account: AccountId,
        delta: i64,
        belongings: Option<Belongings>,
    },
    /// About the marketplace, for player `peer` (answered with `Answer::Market`).
    Market {
        peer: u32,
        ask: MarketJob,
    },
    /// Answers once everything asked before is done.
    Flush(Sender<()>),
}

/// What a player asks of the marketplace.
pub enum MarketJob {
    Offers,
    Offer {
        seller: AccountId,
        seller_name: String,
        what: &'static str,
        data: String,
        price: i64,
        belongings: Belongings,
    },
    Withdraw {
        seller: AccountId,
        offer: i64,
    },
    Buy {
        buyer: Option<AccountId>,
        offer: i64,
        price: i64,
    },
}

/// What the database thread says back.
pub enum Answer {
    /// To a login: for whom (the server's player number), and who came in or why not.
    Login {
        peer: u32,
        login: Result<Login, String>,
    },
    /// What's in the wallets of the accounts playing here (after `refresh`).
    Balances(Vec<(AccountId, i64)>),
    /// To what player `peer` asked of the marketplace.
    Market { peer: u32, reply: MarketReply },
}

/// The database as a server uses it: a thread of its own runs what it's asked, in order.
/// It ends when this is dropped, once it's done.
pub struct Database {
    jobs: Sender<Job>,
    answers: Mutex<Receiver<Answer>>,
}

impl Database {
    /// Opens the database at `url` (see `Store::open`) on a thread of its own, waiting
    /// until it's open. `instance` names this server among the ones sharing it.
    pub fn open(url: &str, instance: String) -> Result<Self, String> {
        let (jobs, inbox) = channel::<Job>();
        let (answer, answers) = channel::<Answer>();
        let (opened, open) = channel::<Result<(), String>>();
        let url = url.to_string();
        std::thread::Builder::new()
            .name("database".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        let _ = opened.send(Err(e.to_string()));
                        return;
                    }
                };
                let store = match runtime.block_on(Store::open(&url)) {
                    Ok(store) => store,
                    Err(e) => {
                        let _ = opened.send(Err(e));
                        return;
                    }
                };
                let _ = opened.send(Ok(()));
                for job in inbox {
                    runtime.block_on(run(&store, &instance, job, &answer));
                }
                runtime.block_on(store.pool.close());
            })
            .map_err(|e| e.to_string())?;
        open.recv()
            .map_err(|_| "the database thread ended".to_string())??;
        Ok(Self {
            jobs,
            answers: Mutex::new(answers),
        })
    }

    fn ask(&self, job: Job) {
        // The thread ends only with this.
        let _ = self.jobs.send(job);
    }

    /// Asks to let player `peer` in as `name` with `password`; the answer comes through
    /// `answers`.
    pub fn login(&self, peer: u32, name: &str, password: &str, admission: Admission) {
        self.ask(Job::Login {
            peer,
            name: name.to_string(),
            password: password.to_string(),
            admission,
        });
    }

    /// Saves account `account`'s car.
    pub fn save(&self, account: AccountId, car: String) {
        self.ask(Job::Save { account, car });
    }

    /// Saves everything account `account` owns.
    pub fn save_all(&self, account: AccountId, belongings: Belongings) {
        self.ask(Job::SaveAll {
            account,
            belongings,
        });
    }

    /// Refreshes the locks of the accounts playing here.
    pub fn refresh(&self, accounts: Vec<AccountId>) {
        self.ask(Job::Refresh(accounts));
    }

    /// Lets go of an account's lock (they left).
    pub fn release(&self, account: AccountId) {
        self.ask(Job::Release(account));
    }

    /// Adds `delta` cents to `account`'s wallet, saving `belongings` with it (see
    /// `Store::pay`).
    pub fn pay(&self, account: AccountId, delta: i64, belongings: Option<Belongings>) {
        self.ask(Job::Pay {
            account,
            delta,
            belongings,
        });
    }

    /// Asks something of the marketplace for player `peer`; the answer comes through
    /// `answers`.
    pub fn market(&self, peer: u32, ask: MarketJob) {
        self.ask(Job::Market { peer, ask });
    }

    /// What the database said since this was last asked.
    pub fn answers(&self) -> Vec<Answer> {
        let answers = self.answers.lock().expect("the answers");
        answers.try_iter().collect()
    }

    /// Waits until everything asked so far is done, for at most `timeout`: whether it was.
    pub fn flush(&self, timeout: Duration) -> bool {
        let (done, wait) = channel();
        self.ask(Job::Flush(done));
        wait.recv_timeout(timeout).is_ok()
    }
}

async fn run(store: &Store, instance: &str, job: Job, answer: &Sender<Answer>) {
    let result = match job {
        Job::Login {
            peer,
            name,
            password,
            admission,
        } => {
            let login = store.login(&name, &password, admission, instance).await;
            if let Err(e) = &login {
                bevy::log::info!("Player {peer} not let in as {name}: {e}");
            }
            let _ = answer.send(Answer::Login { peer, login });
            Ok(())
        }
        Job::Save { account, car } => store.save_car(account, &car).await,
        Job::SaveAll {
            account,
            belongings,
        } => store.save_belongings(account, &belongings).await,
        Job::Refresh(accounts) => match store.refresh(instance, &accounts).await {
            Ok(()) => store.balances(&accounts).await.map(|balances| {
                let _ = answer.send(Answer::Balances(balances));
            }),
            Err(e) => Err(e),
        },
        Job::Release(account) => store.release(instance, account).await,
        Job::Pay {
            account,
            delta,
            belongings,
        } => (store.pay(account, delta, belongings.as_ref()).await)
            .map_err(|e| format!("account {account} paying {delta}: {e}")),
        Job::Market { peer, ask } => {
            let reply = match ask {
                MarketJob::Offers => MarketReply::Offers(store.offers().await),
                MarketJob::Offer {
                    seller,
                    seller_name,
                    what,
                    data,
                    price,
                    belongings,
                } => MarketReply::Offered(
                    store
                        .offer((seller, &seller_name), (what, &data, price), &belongings)
                        .await,
                ),
                MarketJob::Withdraw { seller, offer } => {
                    MarketReply::Withdrawn(store.withdraw(seller, offer).await)
                }
                MarketJob::Buy {
                    buyer,
                    offer,
                    price,
                } => MarketReply::Bought(store.buy_offer(buyer, offer, price).await),
            };
            let _ = answer.send(Answer::Market { peer, reply });
            Ok(())
        }
        Job::Flush(done) => {
            let _ = done.send(());
            Ok(())
        }
    };
    if let Err(e) = result {
        bevy::log::warn!("The database: {e}");
    }
}

/// A name for this server instance among the ones sharing a database: random enough.
pub fn instance_name() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(now_ms() as u128);
    hasher.write_u32(std::process::id());
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block<T>(f: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    const ACCOUNTS: Admission = Admission::Accounts { guests: false };
    const GUESTS: Admission = Admission::Accounts { guests: true };

    /// A store at `url` without anything in it (an earlier test's tables dropped).
    async fn fresh(url: &str) -> Store {
        let store = Store::open(url).await.unwrap();
        if store.dialect == Dialect::Sqlite && url.contains(":memory:") {
            return store;
        }
        for table in [
            "accounts",
            "active",
            "cars",
            "items",
            "offers",
            "schema_version",
        ] {
            sqlx::query(AssertSqlSafe(format!("DROP TABLE IF EXISTS {table}")))
                .execute(&store.pool)
                .await
                .unwrap();
        }
        store.pool.close().await;
        Store::open(url).await.unwrap()
    }

    async fn accounts(url: &str) {
        let store = fresh(url).await;
        store.add_user("Anna", "secret").await.unwrap();
        assert!(
            store.add_user("anna", "other").await.is_err(),
            "the same name"
        );
        let anna = store.login("anna", "secret", ACCOUNTS, "a").await.unwrap();
        let Login::Account(anna) = anna else {
            panic!("not an account")
        };
        assert_eq!(anna.name, "Anna");
        assert_eq!(anna.belongings, Belongings::default());
        assert_eq!(
            store.login("Anna", "wrong", ACCOUNTS, "a").await,
            Err(WRONG.into())
        );
        assert_eq!(
            store.login("Ben", "secret", ACCOUNTS, "a").await,
            Err(WRONG.into())
        );
        // Guests only where allowed, and never by an account's name.
        assert!(store.login("Ben", "", ACCOUNTS, "a").await.is_err());
        assert_eq!(
            store.login(" Ben ", "", GUESTS, "a").await,
            Ok(Login::Guest("Ben".into()))
        );
        assert!(store.login("ANNA", "", GUESTS, "a").await.is_err());
        // A new password.
        store.set_password("ANNA", "new").await.unwrap();
        assert!(store.login("Anna", "secret", ACCOUNTS, "a").await.is_err());
        assert!(store.login("Anna", "new", ACCOUNTS, "a").await.is_ok());
        assert_eq!(store.users().await.unwrap().len(), 1);
        store.remove_user("anna").await.unwrap();
        assert!(store.users().await.unwrap().is_empty());
    }

    async fn instances(url: &str) {
        let store = fresh(url).await;
        let id = store.add_user("Anna", "pw").await.unwrap();
        assert!(store.login("Anna", "pw", ACCOUNTS, "one").await.is_ok());
        // Again on the same instance is fine (the server refuses a second game itself).
        assert!(store.login("Anna", "pw", ACCOUNTS, "one").await.is_ok());
        assert!(store.login("Anna", "pw", ACCOUNTS, "two").await.is_err());
        // Left: the other instance may.
        store.release("one", id).await.unwrap();
        assert!(store.login("Anna", "pw", ACCOUNTS, "two").await.is_ok());
        // Gone silent for longer than `STALE`: taken over.
        let old = now_ms() - STALE.as_millis() as i64 - 1000;
        sqlx::query(store.sql("UPDATE active SET refreshed = ?"))
            .bind(old)
            .execute(&store.pool)
            .await
            .unwrap();
        assert!(store.login("Anna", "pw", ACCOUNTS, "one").await.is_ok());
        // Refreshed, it holds.
        store.refresh("one", &[id]).await.unwrap();
        assert!(store.login("Anna", "pw", ACCOUNTS, "two").await.is_err());
    }

    async fn cars(url: &str) {
        let store = fresh(url).await;
        let alone = store.login("whoever", "", Admission::Alone, "x").await;
        let Ok(Login::Account(account)) = alone else {
            panic!("not let in alone: {alone:?}")
        };
        assert_eq!(account.name, DEFAULT_USER);
        store.save_car(account.id, "first").await.unwrap();
        store
            .save_car(account.id, &"second".repeat(5000))
            .await
            .unwrap();
        let again = store.login("", "", Admission::Alone, "y").await;
        let Ok(Login::Account(again)) = again else {
            panic!("not let in again")
        };
        assert_eq!(again.id, account.id);
        assert_eq!(again.belongings.car, Some("second".repeat(5000)));
        // Everything: the car driven, two in the garage, parts here and there.
        let all = Belongings {
            car: Some("driven".into()),
            garage: vec!["one".into(), "two".into()],
            items: vec![
                (GARAGE.into(), "a part".into()),
                (held_at("Teile Bastei"), "another".into()),
            ],
        };
        store.save_belongings(account.id, &all).await.unwrap();
        assert_eq!(store.belongings(account.id).await.unwrap(), all);
        // The car driven saved alone leaves the rest.
        store.save_car(account.id, "driven on").await.unwrap();
        let after = store.belongings(account.id).await.unwrap();
        assert_eq!(after.car.as_deref(), Some("driven on"));
        assert_eq!(after.garage, all.garage);
        assert_eq!(after.items, all.items);
    }

    async fn wallet(url: &str) {
        let store = fresh(url).await;
        let id = store.add_user("Anna", "pw").await.unwrap();
        let Ok(Login::Account(anna)) = store.login("Anna", "pw", ACCOUNTS, "a").await else {
            panic!("not let in");
        };
        assert_eq!(anna.money, crate::trade::START_MONEY, "a new account's");
        // Paid for with what it bought, in one go.
        let bought = Belongings {
            car: Some("a car".into()),
            garage: Vec::new(),
            items: vec![(GARAGE.into(), "a part bought".into())],
        };
        store.pay(id, -100_000, Some(&bought)).await.unwrap();
        assert_eq!(store.money(id).await.unwrap(), 400_000);
        assert_eq!(store.belongings(id).await.unwrap(), bought);
        // Not enough: neither the money nor what it was for.
        let more = Belongings {
            items: vec![(GARAGE.into(), "too dear".into())],
            ..bought.clone()
        };
        assert!(store.pay(id, -400_001, Some(&more)).await.is_err());
        assert_eq!(store.money(id).await.unwrap(), 400_000);
        assert_eq!(store.belongings(id).await.unwrap(), bought);
        store.pay(id, 2_500, None).await.unwrap();
        assert_eq!(
            store.balances(&[id, id + 99]).await.unwrap(),
            [(id, 402_500)]
        );
    }

    async fn market(url: &str) {
        let store = fresh(url).await;
        let anna = store.add_user("Anna", "pw").await.unwrap();
        let ben = store.add_user("Ben", "pw").await.unwrap();
        let start = crate::trade::START_MONEY;
        // Anna offers a part (gone from her garage with it) and a car.
        let before = Belongings {
            car: Some("her car".into()),
            garage: vec!["a car to sell".into()],
            items: vec![(GARAGE.into(), "a part to sell".into())],
        };
        store.save_belongings(anna, &before).await.unwrap();
        let without = Belongings {
            items: Vec::new(),
            ..before.clone()
        };
        let part = (store.offer((anna, "Anna"), (PART, "a part to sell", 1500), &without))
            .await
            .unwrap();
        assert_eq!(store.belongings(anna).await.unwrap(), without);
        let neither = Belongings {
            garage: Vec::new(),
            ..without.clone()
        };
        let car = (store.offer((anna, "Anna"), (CAR, "a car to sell", 250_000), &neither))
            .await
            .unwrap();
        assert!(
            store
                .offer((anna, "Anna"), ("boat", "?", 1), &neither)
                .await
                .is_err()
        );
        let offers = store.offers().await.unwrap();
        assert_eq!(offers.len(), 2);
        assert_eq!(offers[0].id, part);
        assert_eq!(
            (offers[0].seller, &*offers[0].seller_name, &*offers[0].what),
            (anna, "Anna", PART)
        );
        assert_eq!(offers[1].price, 250_000);
        // Not her own, not at another price, and only once.
        assert!(store.buy_offer(Some(anna), part, 1500).await.is_err());
        assert!(store.buy_offer(Some(ben), part, 1000).await.is_err());
        let bought = store.buy_offer(Some(ben), part, 1500).await.unwrap();
        assert_eq!(bought.data, "a part to sell");
        assert!(store.buy_offer(Some(ben), part, 1500).await.is_err());
        assert_eq!(store.money(ben).await.unwrap(), start - 1500);
        assert_eq!(store.money(anna).await.unwrap(), start + 1500);
        let bens = store.belongings(ben).await.unwrap();
        assert_eq!(
            bens.items,
            [(GARAGE.to_string(), "a part to sell".to_string())]
        );
        // Too dear for Ben: nothing changes.
        store.pay(ben, -(start - 2000), None).await.unwrap();
        assert!(store.buy_offer(Some(ben), car, 250_000).await.is_err());
        assert_eq!(store.offers().await.unwrap().len(), 1);
        assert_eq!(store.money(anna).await.unwrap(), start + 1500);
        // A guest buys (their money and garage the server's): the seller is paid.
        let withdrawn = store.withdraw(ben, car).await;
        assert!(withdrawn.is_err(), "not Ben's to take back");
        let withdrawn = store.withdraw(anna, car).await.unwrap();
        assert_eq!(withdrawn.what, CAR);
        assert!(store.offers().await.unwrap().is_empty());
        assert_eq!(
            store.belongings(anna).await.unwrap().garage,
            ["a car to sell"]
        );
        let again = store.offer((anna, "Anna"), (CAR, "a car to sell", 9900), &neither);
        let again = again.await.unwrap();
        store.buy_offer(None, again, 9900).await.unwrap();
        assert_eq!(store.money(anna).await.unwrap(), start + 1500 + 9900);
        // At most `MOST_OFFERS` each.
        for _ in 0..MOST_OFFERS {
            store
                .offer((ben, "Ben"), (PART, "p", 100), &bens)
                .await
                .unwrap();
        }
        assert!(
            store
                .offer((ben, "Ben"), (PART, "p", 100), &bens)
                .await
                .is_err()
        );
        store.remove_user("Ben").await.unwrap();
        assert!(store.offers().await.unwrap().is_empty());
    }

    const MEMORY: &str = "sqlite::memory:";

    #[test]
    fn offers_are_put_up_bought_and_taken_back() {
        block(market(MEMORY));
    }

    #[test]
    fn a_wallet_is_kept_and_paid_from() {
        block(wallet(MEMORY));
    }

    #[test]
    fn accounts_log_in_by_their_password() {
        block(accounts(MEMORY));
    }

    #[test]
    fn a_player_plays_on_one_instance_at_a_time() {
        block(instances(MEMORY));
    }

    #[test]
    fn a_car_is_saved_and_loaded() {
        block(cars(MEMORY));
    }

    /// The same on the database at `CAR_SIM_TEST_DB` (e.g. `postgres://…`, `mysql://…`),
    /// whose tables it drops.
    #[test]
    #[ignore]
    fn on_another_database() {
        let url = std::env::var("CAR_SIM_TEST_DB").expect("CAR_SIM_TEST_DB");
        block(async {
            accounts(&url).await;
            instances(&url).await;
            cars(&url).await;
            wallet(&url).await;
            market(&url).await;
        });
    }

    #[test]
    fn the_database_thread_answers_logins() {
        let db = Database::open(MEMORY, "test".into()).unwrap();
        db.login(7, "Anna", "", GUESTS);
        assert!(db.flush(Duration::from_secs(10)));
        let answers = db.answers();
        assert_eq!(answers.len(), 1);
        let Answer::Login { peer, login } = &answers[0] else {
            panic!("not a login's answer");
        };
        assert_eq!(*peer, 7);
        assert_eq!(*login, Ok(Login::Guest("Anna".into())));
    }
}
