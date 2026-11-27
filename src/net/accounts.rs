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

/// Where a car or an item is: the car driven, or in the garage.
pub const DRIVEN: &str = "driven";
pub const GARAGE: &str = "garage";

/// The place of an item held at the shop called `shop`.
pub fn held_at(shop: &str) -> String {
    format!("shop {shop}")
}

/// What an account owns as saved: its car driven, the cars in its garage, and loose parts
/// with where each is (`db::GARAGE` or `db::held_at`), all as text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Belongings {
    pub car: Option<String>,
    pub garage: Vec<String>,
    pub items: Vec<(String, String)>,
}

/// What an offer at the marketplace is of (`OfferRow::what`): a part (with what's fitted
/// into it), or a car.
pub const PART: &str = "part";
pub const CAR: &str = "car";

/// An offer at the marketplace as the database keeps it: the part or car as text
/// (`saved::write_items`, `saved::write`).
#[derive(Clone, Debug, PartialEq)]
pub struct OfferRow {
    pub id: i64,
    pub seller: AccountId,
    pub seller_name: String,
    /// `PART` or `CAR`.
    pub what: String,
    pub data: String,
    /// Cents.
    pub price: i64,
}

/// What the database says to what a player asked of the marketplace.
#[derive(Clone, Debug, PartialEq)]
pub enum MarketReply {
    /// The offers there are.
    Offers(Result<Vec<OfferRow>, String>),
    /// Their offer is up (its number), or why not.
    Offered(Result<i64, String>),
    /// Their offer taken back: what it was (in their garage now), or why not.
    Withdrawn(Result<OfferRow, String>),
    /// What they bought (in their garage now, if they have an account; the seller paid), or
    /// why not.
    Bought(Result<OfferRow, String>),
}

/// The most offers a player has up at a time.
pub const MOST_OFFERS: usize = 20;

/// Someone let in: the name they play as, their account (none for a guest) and what they
/// own as saved.
#[derive(Clone, Debug, PartialEq)]
pub struct Admitted {
    pub name: String,
    pub account: Option<AccountId>,
    pub belongings: Belongings,
    /// What's in their wallet (cents).
    pub money: i64,
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::Accounts;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::HashMap;

    use super::*;
    use crate::db::{Admission, Answer, Database, Login, MarketJob};

    /// A server's database and who it lets in.
    pub struct Accounts {
        db: Database,
        admission: Admission,
        /// The names given by the players waiting to be let in.
        waiting: HashMap<PlayerId, String>,
        /// What the database said, but for logins, not yet picked up.
        said: Vec<Answer>,
    }

    impl Accounts {
        /// Accounts in the database at `url`, and guests if `guests`.
        pub fn open(url: &str, guests: bool) -> Result<Self, String> {
            Ok(Self {
                db: Database::open(url, crate::db::instance_name())?,
                admission: Admission::Accounts { guests },
                waiting: HashMap::new(),
                said: Vec::new(),
            })
        }

        /// Playing alone, in the database at `url`: whoever comes is the default user, and
        /// plays by the name they give.
        pub fn alone(url: &str) -> Result<Self, String> {
            Ok(Self {
                db: Database::open(url, "alone".into())?,
                admission: Admission::Alone,
                waiting: HashMap::new(),
                said: Vec::new(),
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
            self.said.extend(self.db.answers());
            let mut logins = Vec::new();
            self.said.retain_mut(|answer| match answer {
                Answer::Login { peer, login } => {
                    let login = std::mem::replace(login, Err(String::new()));
                    logins.push((*peer, login));
                    false
                }
                _ => true,
            });
            logins
                .into_iter()
                .map(|(peer, login)| {
                    let given = self.waiting.remove(&peer).unwrap_or_default();
                    let admitted = login.map(|login| match login {
                        Login::Account(account) => Admitted {
                            // Alone, by the name the game gives.
                            name: if alone && !given.is_empty() {
                                given
                            } else {
                                account.name
                            },
                            account: Some(account.id),
                            belongings: account.belongings,
                            money: account.money,
                        },
                        Login::Guest(name) => Admitted {
                            name,
                            account: None,
                            belongings: Belongings::default(),
                            money: crate::trade::START_MONEY,
                        },
                    });
                    (peer, admitted)
                })
                .collect()
        }

        /// The wallets of the accounts playing here as the database has them, told since
        /// this was last asked (after `refresh`), in order.
        pub fn balances(&mut self) -> Vec<Vec<(AccountId, i64)>> {
            // (Logins that came meanwhile wait for `answers`.)
            self.said.extend(self.db.answers());
            let mut balances = Vec::new();
            self.said.retain_mut(|answer| match answer {
                Answer::Balances(told) => {
                    balances.push(std::mem::take(told));
                    false
                }
                _ => true,
            });
            balances
        }

        /// Adds `delta` cents to `account`'s wallet, saving everything it owns
        /// (`belongings`, if given) with it, in one go.
        pub fn pay(&self, account: AccountId, delta: i64, belongings: Option<Belongings>) {
            self.db.pay(account, delta, belongings);
        }

        /// Asks for the offers at the marketplace, for player `peer`.
        pub fn offers(&self, peer: PlayerId) {
            self.db.market(peer, MarketJob::Offers);
        }

        /// Puts up an offer for player `peer` (account `seller`, called `seller_name`):
        /// `data` (what `what` says it is) for `price` cents, with what they own without it
        /// (`belongings`).
        pub fn offer(
            &self,
            peer: PlayerId,
            (seller, seller_name): (AccountId, String),
            (what, data, price): (&'static str, String, i64),
            belongings: Belongings,
        ) {
            let ask = MarketJob::Offer {
                seller,
                seller_name,
                what,
                data,
                price,
                belongings,
            };
            self.db.market(peer, ask);
        }

        /// Player `peer` (account `seller`) takes their offer back.
        pub fn withdraw(&self, peer: PlayerId, seller: AccountId, offer: i64) {
            self.db.market(peer, MarketJob::Withdraw { seller, offer });
        }

        /// Player `peer` (account `buyer`, none for a guest) buys an offer for `price`.
        pub fn buy_offer(&self, peer: PlayerId, buyer: Option<AccountId>, offer: i64, price: i64) {
            let ask = MarketJob::Buy {
                buyer,
                offer,
                price,
            };
            self.db.market(peer, ask);
        }

        /// The database's answers about the marketplace since this was last asked, for
        /// whom (only player `only`'s, if given: the others' wait).
        pub fn market(&mut self, only: Option<PlayerId>) -> Vec<(PlayerId, MarketReply)> {
            self.said.extend(self.db.answers());
            let mut replies = Vec::new();
            self.said.retain_mut(|answer| match answer {
                Answer::Market { peer, reply } if only.is_none_or(|only| only == *peer) => {
                    let reply = std::mem::replace(reply, MarketReply::Offers(Ok(Vec::new())));
                    replies.push((*peer, reply));
                    false
                }
                _ => true,
            });
            replies
        }

        /// Saves `account`'s car driven (text, `saved::write`).
        pub fn save(&self, account: AccountId, car: String) {
            self.db.save(account, car);
        }

        /// Saves everything `account` owns.
        pub fn save_all(&self, account: AccountId, belongings: Belongings) {
            self.db.save_all(account, belongings);
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
    pub fn save_all(&self, _: AccountId, _: Belongings) {}
    pub fn refresh(&self, _: Vec<AccountId>) {}
    pub fn release(&self, _: AccountId) {}
    pub fn balances(&mut self) -> Vec<Vec<(AccountId, i64)>> {
        Vec::new()
    }
    pub fn pay(&self, _: AccountId, _: i64, _: Option<Belongings>) {}
    pub fn offers(&self, _: PlayerId) {}
    pub fn offer(
        &self,
        _: PlayerId,
        _: (AccountId, String),
        _: (&'static str, String, i64),
        _: Belongings,
    ) {
    }
    pub fn withdraw(&self, _: PlayerId, _: AccountId, _: i64) {}
    pub fn buy_offer(&self, _: PlayerId, _: Option<AccountId>, _: i64, _: i64) {}
    pub fn market(&mut self, _: Option<PlayerId>) -> Vec<(PlayerId, MarketReply)> {
        Vec::new()
    }
    pub fn flush(&self, _: Duration) -> bool {
        true
    }
}
