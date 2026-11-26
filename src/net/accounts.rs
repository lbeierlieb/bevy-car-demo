//! Who a server lets in, and keeps things for: its database (`db`), as the server's
//! systems use it. A server without one (`Server::accounts` none: tests, and a dedicated
//! server not given a database) lets everyone in by the name they give, and keeps nothing.
//! The web game never runs a server, so there it's only the type, never made.

use std::time::Duration;

use super::protocol::PlayerId;

/// An account's number in the database.
pub type AccountId = i64;

/// How often the accounts playing are told to the database (see `db::REFRESH`).
#[cfg(not(target_arch = "wasm32"))]
pub const REFRESH: Duration = crate::db::REFRESH;
#[cfg(target_arch = "wasm32")]
pub const REFRESH: Duration = Duration::from_secs(10);

/// Someone let in: the name they play as, their account (none for a guest) and their car
/// as saved (if any).
#[derive(Clone, Debug, PartialEq)]
pub struct Admitted {
    pub name: String,
    pub account: Option<AccountId>,
    pub car: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::Accounts;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::HashMap;

    use super::*;
    use crate::db::{Admission, Database, Login};

    /// A server's database and who it lets in.
    pub struct Accounts {
        db: Database,
        admission: Admission,
        /// The names given by the players waiting to be let in.
        waiting: HashMap<PlayerId, String>,
    }

    impl Accounts {
        /// Accounts in the database at `url`, and guests if `guests`.
        pub fn open(url: &str, guests: bool) -> Result<Self, String> {
            Ok(Self {
                db: Database::open(url, crate::db::instance_name())?,
                admission: Admission::Accounts { guests },
                waiting: HashMap::new(),
            })
        }

        /// Playing alone, in the database at `url`: whoever comes is the default user, and
        /// plays by the name they give.
        pub fn alone(url: &str) -> Result<Self, String> {
            Ok(Self {
                db: Database::open(url, "alone".into())?,
                admission: Admission::Alone,
                waiting: HashMap::new(),
            })
        }

        /// Asks to let player `peer` in as `name` with `password`.
        pub fn login(&mut self, peer: PlayerId, name: &str, password: &str) {
            self.waiting.insert(peer, name.trim().to_string());
            self.db.login(peer, name, password, self.admission);
        }

        /// Who was let in since this was last asked, or why not.
        pub fn answers(&mut self) -> Vec<(PlayerId, Result<Admitted, String>)> {
            let alone = self.admission == Admission::Alone;
            let answers = self.db.answers();
            answers
                .into_iter()
                .map(|answer| {
                    let given = self.waiting.remove(&answer.peer).unwrap_or_default();
                    let admitted = answer.login.map(|login| match login {
                        Login::Account(account) => Admitted {
                            // Alone, by the name the game gives.
                            name: if alone && !given.is_empty() {
                                given
                            } else {
                                account.name
                            },
                            account: Some(account.id),
                            car: account.car,
                        },
                        Login::Guest(name) => Admitted {
                            name,
                            account: None,
                            car: None,
                        },
                    });
                    (answer.peer, admitted)
                })
                .collect()
        }

        /// Saves `account`'s car (text, `saved::write`).
        pub fn save(&self, account: AccountId, car: String) {
            self.db.save(account, car);
        }

        /// The accounts playing here are still playing.
        pub fn refresh(&self, accounts: Vec<AccountId>) {
            self.db.refresh(accounts);
        }

        /// `account` left.
        pub fn release(&self, account: AccountId) {
            self.db.release(account);
        }

        /// Waits up to `timeout` for everything asked so far to be done.
        pub fn flush(&self, timeout: Duration) -> bool {
            self.db.flush(timeout)
        }
    }
}

/// The web game has no database (it never runs a server): there's no making one.
#[cfg(target_arch = "wasm32")]
pub struct Accounts(());

#[cfg(target_arch = "wasm32")]
impl Accounts {
    pub fn login(&mut self, _: PlayerId, _: &str, _: &str) {}
    pub fn answers(&mut self) -> Vec<(PlayerId, Result<Admitted, String>)> {
        Vec::new()
    }
    pub fn save(&self, _: AccountId, _: String) {}
    pub fn refresh(&self, _: Vec<AccountId>) {}
    pub fn release(&self, _: AccountId) {}
    pub fn flush(&self, _: Duration) -> bool {
        true
    }
}
