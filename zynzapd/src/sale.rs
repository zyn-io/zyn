//! Selling a place in a Cave collection.
//!
//! # The shape of a sale
//!
//! Three parties sign nothing in common, so the sale is three steps and not
//! one transaction:
//!
//!   1. the buyer pays, by signing [`Intent::FundCollection`] themselves —
//!      their money, their signature, and it lands in the collection's pool
//!      where it backs every outstanding item rather than in anyone's pocket;
//!   2. the allocator decides *which* items, from evidence nobody controls
//!      ([`cave_randomness`]);
//!   3. the creator mints them, threshold-signed by the treasury.
//!
//! There is no atomic paid mint in the VM and this does not invent one. A buyer
//! pays before they hold anything, so like a claim code (§74.5) this service is
//! an operator promise and must not be described as trustless. What it does
//! guarantee is that a paid order is remembered, retrying is safe, and an
//! allocation nobody minted is given back.
//!
//! # Why payment is relayed rather than observed
//!
//! Balances on Zyn are private, so the service cannot look up whether a buyer
//! paid. It does not need to: the buyer hands over a *signed frame*, the
//! service submits it, and acceptance is the proof. That keeps the service
//! holding no key of the buyer's and needing no read it is not entitled to.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// Straight from zyn-vm rather than through swapvm, which only re-exported
// them: `swapvm::fixed::Fixed` *is* `zyn_vm::fixed::Fixed` and both
// `AccountId`s are `[u8; 32]`. Going through the VM crate made the sale look
// bound to one application when it never was — and Cave now mints on
// objectvm, not swapvm (DECISIONS §133.1).
use zyn_vm::fixed::Fixed;
use zyn_vm::spec::AccountId;

pub use crate::claim::{hash_code, CodeHash};

/// How far an order has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Allocation reserved against the invitation; nothing paid yet.
    Reserved,
    /// The buyer's payment was accepted by the chain.
    Paid,
    /// Every serial minted to the buyer. Terminal.
    Minted,
}

/// One purchase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Order {
    pub code_hash: CodeHash,
    pub account: AccountId,
    pub quantity: u32,
    /// What the buyer paid, once they have.
    pub paid: Fixed,
    /// Which items they bought, once the allocator has said.
    pub serials: Vec<u32>,
    pub state: State,
    /// Whether this entry's 90% reached the collection's pool.
    ///
    /// Separate from `state` because it is a *second* chain operation, by a
    /// different signer, after the buyer's payment has already landed. A crash
    /// between the two leaves an entry paid with its pool share unfunded —
    /// silent, and it shorts the redemption floor every other holder is
    /// trusting. Recorded so it can be found and finished (§126).
    pub pool_funded: bool,
}

/// Orders whose buyer paid and whose collection share never landed.
///
/// The thing this makes findable: without it, a failure between the two chain
/// operations is invisible, and shows up later as a redemption floor lower than
/// the entries should have made it, with no way to say whose.
pub fn unfunded(book: &Book) -> Vec<Order> {
    let mut out: Vec<Order> = book
        .orders
        .values()
        .filter(|o| o.state != State::Reserved && !o.pool_funded)
        .cloned()
        .collect();
    out.sort_by_key(|o| o.code_hash);
    out
}

/// Finish the collection's share for one order.
///
/// Idempotent by the same record `settle` writes: an order already marked
/// funded is left alone, so running this twice does not pay twice.
pub fn reconcile(
    book: &mut Book,
    order: &Order,
    fund_pool: &dyn Fn() -> Result<(), String>,
) -> Result<bool, String> {
    let Some(mut current) = book.orders.get(&order.code_hash).cloned() else {
        return Err("no such order".into());
    };
    if current.pool_funded {
        return Ok(false);
    }
    if current.state == State::Reserved {
        return Err("this order was never paid; there is nothing to fund".into());
    }
    fund_pool()?;
    current.pool_funded = true;
    book.orders.insert(current.code_hash, current);
    book.persist()?;
    Ok(true)
}

/// Why a sale was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// No such invitation.
    UnknownCode,
    /// The invitation belongs to another account.
    BoundElsewhere,
    /// The buyer paid, and the collection's share did not reach the pool.
    /// Recoverable: the payment is recorded, so `reconcile` can finish it.
    PoolNotFunded,
    /// The allocation is spent, or the request exceeds what is left.
    Exhausted { remaining: u32 },
    /// The whitelist phase is not open.
    Closed,
    /// The buyer's payment was not what the order costs.
    WrongAmount { expected: Fixed },
    /// The chain refused the payment.
    PaymentRefused,
    /// The collection cannot mint right now.
    NotMinting,
}

/// What a completed sale produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sold {
    pub serials: Vec<u32>,
    pub paid: Fixed,
}

/// The orders, keyed by invitation.
#[derive(Clone, Debug, Default)]
pub struct Book {
    pub orders: BTreeMap<CodeHash, Order>,
    path: Option<PathBuf>,
}

impl Book {
    pub fn new() -> Book {
        Book::default()
    }

    pub fn get(&self, code: &str) -> Option<&Order> {
        self.orders.get(&hash_code(code))
    }

    /// Total sold and total taken, the two numbers an operator watches.
    pub fn totals(&self) -> (u32, Fixed) {
        self.orders.values().fold((0, Fixed::ZERO), |(n, paid), o| {
            (
                n.saturating_add(o.serials.len() as u32),
                paid.add(o.paid).unwrap_or(paid),
            )
        })
    }

    pub fn encode(&self) -> String {
        let mut s = String::from("# zyn cave sale v1\n# code_hash\taccount\tqty\tpaid\tstate\tserials\tpool\n");
        for o in self.orders.values() {
            s.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                hex(&o.code_hash),
                hex(&o.account),
                o.quantity,
                o.paid,
                match o.state {
                    State::Reserved => "reserved",
                    State::Paid => "paid",
                    State::Minted => "minted",
                },
                if o.serials.is_empty() {
                    "-".to_string()
                } else {
                    o.serials
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                },
                if o.pool_funded { "funded" } else { "pending" },
            ));
        }
        s
    }

    pub fn decode(text: &str) -> Result<Book, String> {
        let mut b = Book::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim_end();
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 6 && f.len() != 7 {
                return Err(format!("line {}: expected six or seven fields", n + 1));
            }
            // A record written before the pool field existed is treated as
            // unfunded. Reconciliation then checks it rather than trusting it.
            let pool_funded = f.len() == 7 && f[6] == "funded";
            let code_hash = unhex32(f[0]).ok_or_else(|| format!("line {}: bad code hash", n + 1))?;
            let account = unhex32(f[1]).ok_or_else(|| format!("line {}: bad account", n + 1))?;
            let quantity: u32 = f[2]
                .parse()
                .map_err(|_| format!("line {}: quantity is not a number", n + 1))?;
            let paid = crate::client::fixed_of(f[3])
                .map_err(|_| format!("line {}: paid is not an amount", n + 1))?;
            let state = match f[4] {
                "reserved" => State::Reserved,
                "paid" => State::Paid,
                "minted" => State::Minted,
                other => return Err(format!("line {}: unknown state {}", n + 1, other)),
            };
            let serials = if f[5] == "-" {
                Vec::new()
            } else {
                f[5].split(',')
                    .map(|s| s.parse::<u32>())
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| format!("line {}: serials are not numbers", n + 1))?
            };
            b.orders.insert(
                code_hash,
                Order {
                    pool_funded,
                    code_hash,
                    account,
                    quantity,
                    paid,
                    serials,
                    state,
                },
            );
        }
        Ok(b)
    }

    pub fn load(path: &Path) -> Result<Book, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let mut b = Book::decode(&text)?;
        b.path = Some(path.to_path_buf());
        Ok(b)
    }

    /// Written through a temporary file and renamed, so an interrupted save
    /// leaves the previous book rather than half of this one.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.encode()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    pub fn persist(&self) -> Result<(), String> {
        match &self.path {
            Some(p) => self.save(p),
            None => Ok(()),
        }
    }

    pub fn set_path(&mut self, p: PathBuf) {
        self.path = Some(p);
    }
}

fn hex(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// What a buyer is quoted before they sign anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quote {
    pub quantity: u32,
    pub unit_price: Fixed,
    pub total: Fixed,
}

/// Reserve an invitation's allocation and price it.
///
/// Reserving before the buyer pays is deliberate: two people holding the same
/// invitation must not both be quoted the last item. A quote that is never paid
/// is released by [`abandon`], so a reservation is a hold and not a sale.
pub fn quote(
    registry: &mut crate::whitelist::Registry,
    book: &mut Book,
    code: &str,
    account: AccountId,
    quantity: u32,
    unit_price: Fixed,
) -> Result<Quote, Refused> {
    use crate::whitelist::Refused as W;

    // An order already under way answers with what it already is, rather than
    // reserving a second time against the same invitation.
    if let Some(existing) = book.get(code) {
        if existing.account != account {
            return Err(Refused::BoundElsewhere);
        }
        if existing.state != State::Reserved {
            return Ok(Quote {
                quantity: existing.quantity,
                unit_price,
                total: existing.paid,
            });
        }
    }

    let entry = registry
        .reserve(code, account, quantity)
        .map_err(|e| match e {
            W::Unknown => Refused::UnknownCode,
            W::BoundElsewhere => Refused::BoundElsewhere,
            W::Closed => Refused::Closed,
            W::Exhausted { remaining } => Refused::Exhausted { remaining },
        })?;
    registry.persist().ok();

    let total = unit_price
        .mul(Fixed::whole(quantity as i64))
        .ok_or(Refused::WrongAmount { expected: Fixed::ZERO })?;

    book.orders.insert(
        entry.code_hash,
        Order {
            // A new reservation has paid nothing, so its pool share is pending.
            pool_funded: false,
            code_hash: entry.code_hash,
            account,
            quantity,
            paid: Fixed::ZERO,
            serials: Vec::new(),
            state: State::Reserved,
        },
    );
    book.persist().ok();

    Ok(Quote {
        quantity,
        unit_price,
        total,
    })
}

/// Give back a reservation the buyer never paid for.
///
/// The invitation stays bound to them — it touched their account and belongs to
/// it from then on (§74.15) — but the allocation returns so it is not held for
/// ever by someone who walked away.
pub fn abandon(registry: &mut crate::whitelist::Registry, book: &mut Book, code: &str) {
    let Some(order) = book.get(code).cloned() else {
        return;
    };
    if order.state != State::Reserved {
        return; // paid or minted: not ours to take back
    }
    registry.release(code, order.quantity);
    registry.persist().ok();
    book.orders.remove(&order.code_hash);
    book.persist().ok();
}

/// Take payment, allocate and mint.
///
/// The steps are ordered so that every crash leaves something a retry can
/// finish, and none leaves a buyer who paid with nothing to show:
///
///   * payment is submitted first and recorded the moment it is accepted — a
///     crash after this loses no money, because the order says `Paid` and the
///     retry skips straight to minting;
///   * the allocation is written before any mint, so a retry mints the *same*
///     serials rather than drawing new ones;
///   * minting is idempotent per serial: an item already at the buyer's account
///     is success, not a duplicate.
///
/// `pay` submits the buyer's signed frame and returns what the chain took.
/// `allocate` chooses serials. `mint` moves one to the buyer. All three are
/// passed in so the ordering can be tested without a chain or a treasury.
#[allow(clippy::too_many_arguments)]
pub fn settle(
    book: &mut Book,
    code: &str,
    account: AccountId,
    expected: Fixed,
    pay: &dyn Fn() -> Result<Fixed, String>,
    // Forward the pool's share. Called *after* the buyer's payment is recorded
    // as landed, never in the same step — see `Order::pool_funded`.
    fund_pool: &dyn Fn() -> Result<(), String>,
    allocate: &dyn Fn(u32) -> Result<Vec<u32>, String>,
    mint: &dyn Fn(u32, AccountId) -> Result<(), String>,
) -> Result<Sold, Refused> {
    let Some(mut order) = book.get(code).cloned() else {
        return Err(Refused::UnknownCode);
    };
    if order.account != account {
        return Err(Refused::BoundElsewhere);
    }
    if order.state == State::Minted {
        return Ok(Sold {
            serials: order.serials,
            paid: order.paid,
        });
    }

    if order.state == State::Reserved {
        let taken = pay().map_err(|_| Refused::PaymentRefused)?;
        if taken != expected {
            return Err(Refused::WrongAmount { expected });
        }
        // Written down before the pool is funded, and that order is the whole
        // point: the buyer's money has moved, so a failure after this line must
        // leave a record that says so. Rolling back to Reserved would lose the
        // fact that they paid.
        order.paid = taken;
        order.state = State::Paid;
        book.orders.insert(order.code_hash, order.clone());
        book.persist().map_err(|_| Refused::PaymentRefused)?;
    }

    // The collection's share. A failure here is recoverable precisely because
    // the line above already committed — `reconcile` finds this order and
    // finishes it rather than a holder discovering a short floor later.
    if !order.pool_funded {
        fund_pool().map_err(|_| Refused::PoolNotFunded)?;
        order.pool_funded = true;
        book.orders.insert(order.code_hash, order.clone());
        book.persist().map_err(|_| Refused::PoolNotFunded)?;
    }

    if order.serials.is_empty() {
        order.serials = allocate(order.quantity).map_err(|_| Refused::NotMinting)?;
        book.orders.insert(order.code_hash, order.clone());
        book.persist().map_err(|_| Refused::NotMinting)?;
    }

    for serial in &order.serials {
        mint(*serial, account).map_err(|_| Refused::NotMinting)?;
    }
    order.state = State::Minted;
    book.orders.insert(order.code_hash, order.clone());
    book.persist().map_err(|_| Refused::NotMinting)?;

    Ok(Sold {
        serials: order.serials,
        paid: order.paid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::whitelist::Registry;
    use std::cell::RefCell;

    const A: AccountId = [1u8; 32];
    const B: AccountId = [2u8; 32];
    fn price() -> Fixed {
        Fixed::whole(2)
    }

    fn registry_with(code: &str, allocation: u32) -> Registry {
        let mut r = Registry::new();
        r.open = true;
        r.issue(code, allocation);
        r
    }

    /// A chain that counts what it was asked to do.
    struct Chain {
        payments: RefCell<u32>,
        allocations: RefCell<u32>,
        minted: RefCell<Vec<(u32, AccountId)>>,
        pay_fails: RefCell<bool>,
        pool_fails: RefCell<bool>,
        poolings: RefCell<u32>,
        mint_fails_after: RefCell<Option<usize>>,
    }
    impl Chain {
        fn new() -> Chain {
            Chain {
                payments: RefCell::new(0),
                allocations: RefCell::new(0),
                minted: RefCell::new(Vec::new()),
                pay_fails: RefCell::new(false),
                pool_fails: RefCell::new(false),
                poolings: RefCell::new(0),
                mint_fails_after: RefCell::new(None),
            }
        }
        fn pay(&self, amount: Fixed) -> impl Fn() -> Result<Fixed, String> + '_ {
            move || {
                if *self.pay_fails.borrow() {
                    return Err("refused".into());
                }
                *self.payments.borrow_mut() += 1;
                Ok(amount)
            }
        }
        fn fund_pool(&self) -> impl Fn() -> Result<(), String> + '_ {
            move || {
                if *self.pool_fails.borrow() {
                    return Err("the pool was not funded".into());
                }
                *self.poolings.borrow_mut() += 1;
                Ok(())
            }
        }
        fn allocate(&self) -> impl Fn(u32) -> Result<Vec<u32>, String> + '_ {
            move |n| {
                let round = *self.allocations.borrow();
                *self.allocations.borrow_mut() += 1;
                // Different serials each round, so a second allocation is
                // visible rather than silently identical.
                Ok((0..n).map(|i| round * 100 + i).collect())
            }
        }
        fn mint(&self) -> impl Fn(u32, AccountId) -> Result<(), String> + '_ {
            move |serial, to| {
                if let Some(limit) = *self.mint_fails_after.borrow() {
                    if self.minted.borrow().len() >= limit {
                        return Err("mint refused".into());
                    }
                }
                self.minted.borrow_mut().push((serial, to));
                Ok(())
            }
        }
    }

    #[test]
    fn a_quote_reserves_the_allocation_so_two_buyers_cannot_take_the_last_item() {
        let mut r = registry_with("invite", 2);
        let mut b = Book::new();
        let q = quote(&mut r, &mut b, "invite", A, 2, price()).unwrap();
        assert_eq!(q.total, Fixed::whole(4));
        assert_eq!(r.lookup("invite").unwrap().remaining(), 0, "held, not sold");
        assert_eq!(
            quote(&mut r, &mut b, "invite", B, 1, price()),
            Err(Refused::BoundElsewhere)
        );
    }

    #[test]
    fn a_full_sale_pays_once_allocates_once_and_mints_each_serial() {
        let mut r = registry_with("invite", 3);
        let mut b = Book::new();
        let c = Chain::new();
        quote(&mut r, &mut b, "invite", A, 3, price()).unwrap();
        let sold = settle(
            &mut b, "invite", A, Fixed::whole(6),
            &c.pay(Fixed::whole(6)), &c.fund_pool(), &c.allocate(), &c.mint(),
        )
        .unwrap();
        assert_eq!(sold.serials.len(), 3);
        assert_eq!(sold.paid, Fixed::whole(6));
        assert_eq!(*c.payments.borrow(), 1);
        assert_eq!(*c.allocations.borrow(), 1);
        assert_eq!(c.minted.borrow().len(), 3);
        assert!(c.minted.borrow().iter().all(|(_, to)| *to == A));
        assert_eq!(b.get("invite").unwrap().state, State::Minted);
    }

    /// The crash that would cost a buyer money: paid, then died. The retry must
    /// not charge again.
    #[test]
    fn a_retry_after_payment_does_not_charge_twice() {
        let mut r = registry_with("invite", 1);
        let mut b = Book::new();
        let c = Chain::new();
        quote(&mut r, &mut b, "invite", A, 1, price()).unwrap();

        // Paid, then minting fails — the shape of a crash after payment.
        *c.mint_fails_after.borrow_mut() = Some(0);
        assert!(settle(&mut b, "invite", A, Fixed::whole(2), &c.pay(Fixed::whole(2)), &c.fund_pool(), &c.allocate(), &c.mint()).is_err());
        assert_eq!(b.get("invite").unwrap().state, State::Paid, "the money is remembered");

        // Retry: no second payment, and it completes.
        *c.mint_fails_after.borrow_mut() = None;
        let sold = settle(&mut b, "invite", A, Fixed::whole(2), &c.pay(Fixed::whole(2)), &c.fund_pool(), &c.allocate(), &c.mint()).unwrap();
        assert_eq!(*c.payments.borrow(), 1, "charged exactly once");
        assert_eq!(sold.paid, Fixed::whole(2));
    }

    /// A retry must mint the serials the buyer was allocated, not new ones.
    #[test]
    fn a_retry_mints_the_same_serials_it_allocated() {
        let mut r = registry_with("invite", 2);
        let mut b = Book::new();
        let c = Chain::new();
        quote(&mut r, &mut b, "invite", A, 2, price()).unwrap();

        *c.mint_fails_after.borrow_mut() = Some(1);       // one lands, one fails
        assert!(settle(&mut b, "invite", A, Fixed::whole(4), &c.pay(Fixed::whole(4)), &c.fund_pool(), &c.allocate(), &c.mint()).is_err());
        let first = b.get("invite").unwrap().serials.clone();
        assert_eq!(first.len(), 2);

        *c.mint_fails_after.borrow_mut() = None;
        let sold = settle(&mut b, "invite", A, Fixed::whole(4), &c.pay(Fixed::whole(4)), &c.fund_pool(), &c.allocate(), &c.mint()).unwrap();
        assert_eq!(sold.serials, first, "the same items, not a fresh draw");
        assert_eq!(*c.allocations.borrow(), 1, "allocated exactly once");
    }

    #[test]
    fn a_completed_sale_answers_identically_however_often_it_is_asked() {
        let mut r = registry_with("invite", 1);
        let mut b = Book::new();
        let c = Chain::new();
        quote(&mut r, &mut b, "invite", A, 1, price()).unwrap();
        let first = settle(&mut b, "invite", A, Fixed::whole(2), &c.pay(Fixed::whole(2)), &c.fund_pool(), &c.allocate(), &c.mint()).unwrap();
        let again = settle(&mut b, "invite", A, Fixed::whole(2), &c.pay(Fixed::whole(2)), &c.fund_pool(), &c.allocate(), &c.mint()).unwrap();
        assert_eq!(first, again);
        assert_eq!(c.minted.borrow().len(), 1, "minted once");
        assert_eq!(*c.payments.borrow(), 1, "paid once");
    }

    /// Paying the wrong amount buys nothing, and is not recorded as paid.
    #[test]
    fn an_underpayment_is_refused_and_leaves_the_order_reserved() {
        let mut r = registry_with("invite", 2);
        let mut b = Book::new();
        let c = Chain::new();
        quote(&mut r, &mut b, "invite", A, 2, price()).unwrap();
        assert_eq!(
            settle(&mut b, "invite", A, Fixed::whole(4), &c.pay(Fixed::whole(1)), &c.fund_pool(), &c.allocate(), &c.mint()),
            Err(Refused::WrongAmount { expected: Fixed::whole(4) })
        );
        assert_eq!(b.get("invite").unwrap().state, State::Reserved);
        assert!(c.minted.borrow().is_empty());
    }

    #[test]
    fn a_refused_payment_mints_nothing() {
        let mut r = registry_with("invite", 1);
        let mut b = Book::new();
        let c = Chain::new();
        quote(&mut r, &mut b, "invite", A, 1, price()).unwrap();
        *c.pay_fails.borrow_mut() = true;
        assert_eq!(
            settle(&mut b, "invite", A, Fixed::whole(2), &c.pay(Fixed::whole(2)), &c.fund_pool(), &c.allocate(), &c.mint()),
            Err(Refused::PaymentRefused)
        );
        assert!(c.minted.borrow().is_empty());
        assert_eq!(*c.allocations.borrow(), 0, "nothing allocated for an unpaid order");
    }

    /// A buyer who walks away must not hold the allocation for ever — but the
    /// invitation stays theirs (§74.15).
    #[test]
    fn abandoning_returns_the_allocation_and_keeps_the_binding() {
        let mut r = registry_with("invite", 2);
        let mut b = Book::new();
        quote(&mut r, &mut b, "invite", A, 2, price()).unwrap();
        assert_eq!(r.lookup("invite").unwrap().remaining(), 0);

        abandon(&mut r, &mut b, "invite");
        assert_eq!(r.lookup("invite").unwrap().remaining(), 2, "given back");
        assert_eq!(
            r.lookup("invite").unwrap().bound_to,
            Some(crate::whitelist::bind(&r.salt, &A)),
            "still theirs — the registry keeps a commitment, never the account"
        );
        assert_eq!(
            quote(&mut r, &mut b, "invite", B, 1, price()),
            Err(Refused::BoundElsewhere)
        );
    }

    #[test]
    fn a_paid_order_cannot_be_abandoned() {
        let mut r = registry_with("invite", 1);
        let mut b = Book::new();
        let c = Chain::new();
        quote(&mut r, &mut b, "invite", A, 1, price()).unwrap();
        settle(&mut b, "invite", A, Fixed::whole(2), &c.pay(Fixed::whole(2)), &c.fund_pool(), &c.allocate(), &c.mint()).unwrap();
        abandon(&mut r, &mut b, "invite");
        assert!(b.get("invite").is_some(), "a sale is not undone by walking away");
    }

    #[test]
    fn the_book_round_trips() {
        let mut r = registry_with("invite", 2);
        let mut b = Book::new();
        let c = Chain::new();
        quote(&mut r, &mut b, "invite", A, 2, price()).unwrap();
        settle(&mut b, "invite", A, Fixed::whole(4), &c.pay(Fixed::whole(4)), &c.fund_pool(), &c.allocate(), &c.mint()).unwrap();
        let back = Book::decode(&b.encode()).expect("decode");
        let o = back.get("invite").unwrap();
        assert_eq!(o.state, State::Minted);
        assert_eq!(o.paid, Fixed::whole(4));
        assert_eq!(o.serials.len(), 2);
        assert_eq!(o.account, A);
        assert_eq!(back.totals(), (2, Fixed::whole(4)));
    }

    #[test]
    fn a_malformed_book_is_refused_rather_than_half_read() {
        assert!(Book::decode("nothex\tx\t1\t1\treserved\t-").is_err());
        assert!(Book::decode(&format!("{}\t{}\t1\t1\tbogus\t-", "ab".repeat(32), "cd".repeat(32))).is_err());
    }
    /// The failure this split exists for.
    ///
    /// The buyer's money has already moved when the pool funding fails. What
    /// must not happen is the order rolling back to Reserved, because then
    /// nothing records that they paid and the shortfall is invisible — it shows
    /// up later as a redemption floor lower than the entries should have made
    /// it, with no way to say whose.
    #[test]
    fn a_pool_that_fails_to_fund_leaves_the_payment_recorded() {
        let mut b = Book::new();
        let mut r = registry_with("invite", 1);
        quote(&mut r, &mut b, "invite", A, 1, Fixed::whole(2)).unwrap();
        let c = Chain::new();

        *c.pool_fails.borrow_mut() = true;
        let refused = settle(&mut b, "invite", A, Fixed::whole(2), &c.pay(Fixed::whole(2)), &c.fund_pool(), &c.allocate(), &c.mint());
        assert_eq!(refused, Err(Refused::PoolNotFunded));

        let order = b.get("invite").unwrap();
        assert_eq!(order.state, State::Paid, "the payment is recorded, not rolled back");
        assert!(!order.pool_funded, "and the missing share is recorded too");
        assert_eq!(*c.payments.borrow(), 1);

        // Reconciling: the same settle, with the pool reachable again. The
        // buyer must not be charged a second time.
        *c.pool_fails.borrow_mut() = false;
        let sold = settle(&mut b, "invite", A, Fixed::whole(2), &c.pay(Fixed::whole(2)), &c.fund_pool(), &c.allocate(), &c.mint()).unwrap();
        assert_eq!(*c.payments.borrow(), 1, "the buyer paid once, not twice");
        assert_eq!(*c.poolings.borrow(), 1, "and the pool got its share exactly once");
        assert_eq!(b.get("invite").unwrap().state, State::Minted);
        assert!(b.get("invite").unwrap().pool_funded);
        assert_eq!(sold.serials.len(), 1);
    }

    /// A book written before the pool field existed reads as unfunded, so it is
    /// checked rather than assumed to have been paid out.
    #[test]
    fn an_older_record_is_read_as_not_yet_funded() {
        let line = format!("{}\t{}\t1\t2\tpaid\t7\n", hex(&[0x11u8; 32]), hex(&A));
        let b = Book::decode(&line).unwrap();
        let order = b.orders.values().next().unwrap();
        assert_eq!(order.state, State::Paid);
        assert!(!order.pool_funded, "six fields means the share is unaccounted for");
    }

    /// Reconciliation is idempotent: an operator who runs it twice, or who
    /// runs it while a retried settle already finished the job, must not fund
    /// the pool a second time.
    #[test]
    fn reconciling_finds_the_shortfall_once_and_only_once() {
        let mut b = Book::new();
        let mut r = registry_with("invite", 1);
        quote(&mut r, &mut b, "invite", A, 1, Fixed::whole(2)).unwrap();
        let c = Chain::new();

        *c.pool_fails.borrow_mut() = true;
        let _ = settle(&mut b, "invite", A, Fixed::whole(2), &c.pay(Fixed::whole(2)), &c.fund_pool(), &c.allocate(), &c.mint());

        let owed = unfunded(&b);
        assert_eq!(owed.len(), 1, "the shortfall is findable");

        *c.pool_fails.borrow_mut() = false;
        assert!(reconcile(&mut b, &owed[0], &c.fund_pool()).unwrap(), "it funds");
        assert!(!reconcile(&mut b, &owed[0], &c.fund_pool()).unwrap(), "and does not fund again");
        assert_eq!(*c.poolings.borrow(), 1);
        assert!(unfunded(&b).is_empty(), "nothing is owed afterwards");
    }

    /// A reservation nobody paid for is not a shortfall — funding its share
    /// would be paying the pool out of the treasury for a sale that never was.
    #[test]
    fn an_unpaid_reservation_is_not_reconciled() {
        let mut b = Book::new();
        let mut r = registry_with("invite", 1);
        quote(&mut r, &mut b, "invite", A, 1, Fixed::whole(2)).unwrap();
        let c = Chain::new();
        assert!(unfunded(&b).is_empty(), "a reservation owes the pool nothing");
        let order = b.get("invite").unwrap().clone();
        assert!(reconcile(&mut b, &order, &c.fund_pool()).is_err());
        assert_eq!(*c.poolings.borrow(), 0);
    }

}
