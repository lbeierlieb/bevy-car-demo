//! Money: every player has a wallet (an account's is kept in the database, a guest's for the
//! session), in cents. It changes only on the server, in one go with what it pays for (see
//! `net::stash` and `db`); the game is told it with what the player owns
//! (`OwnedView::money`).

/// What a new player has (cents): an account when it's made, a guest when they join.
pub const START_MONEY: i64 = 500_000;

/// An amount for people: "5 000 €", "12.50 €", "−300 €".
pub fn euros(cents: i64) -> String {
    let sign = if cents < 0 { "−" } else { "" };
    let cents = cents.unsigned_abs();
    let whole = (cents / 100).to_string();
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    match cents % 100 {
        0 => format!("{sign}{grouped} €"),
        rest => format!("{sign}{grouped}.{rest:02} €"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_read_well() {
        assert_eq!(euros(START_MONEY), "5 000 €");
        assert_eq!(euros(0), "0 €");
        assert_eq!(euros(1250), "12.50 €");
        assert_eq!(euros(-30_000), "−300 €");
        assert_eq!(euros(123_456_789), "1 234 567.89 €");
    }
}
