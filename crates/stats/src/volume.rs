//! Observed volume — the one row of `docs/SPEC.md` §6.2 phase 2 needs.
//!
//! The rest of §6.2 (fiat volume, tickets, size buckets, the inferred
//! conversion) is roadmap PR 34. What the bestiary (§6.5) and the summary
//! (§6.10) need before then is the sats volume alone: the sum of
//! `amount_sats` over orders completed in the window, dated by
//! `success_at` like every completed-side figure in [`crate::activity`].

use crate::activity::{Order, Status};
use crate::window::Window;

/// `∑ amount_sats` of the orders that reached `success` in `window`.
pub fn observed_sats(orders: &[Order], window: Window) -> i64 {
    orders
        .iter()
        .filter(|order| order.status == Status::Success)
        .filter(|order| order.success_at.is_some_and(|at| window.contains(at)))
        .map(|order| order.amount_sats)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::Direction;

    fn order(id: &str, status: Status, success_at: Option<i64>, amount_sats: i64) -> Order {
        Order {
            order_id: id.to_string(),
            pubkey: "pk".into(),
            instance: "pk".into(),
            created_at: 500,
            status,
            direction: Direction::Buy,
            fiat_code: "ARS".into(),
            payment_methods: vec![],
            amount_sats,
            taken_at: None,
            success_at,
            canceled_at: None,
            expires_at: None,
        }
    }

    #[test]
    fn volume_sums_the_orders_completed_in_the_window() {
        // Arrange
        let orders = vec![
            order("in", Status::Success, Some(1_500), 100),
            order("also", Status::Success, Some(1_999), 200),
            order("before", Status::Success, Some(999), 400),
            order("open", Status::Pending, None, 800),
            // Canceled after a taker: never completed, whatever it was worth.
            order("gone", Status::Canceled, None, 1_600),
        ];

        // Act
        let volume = observed_sats(&orders, Window::new(1_000, 2_000));

        // Assert
        assert_eq!(volume, 300);
    }

    #[test]
    fn no_completed_orders_is_zero_volume_not_a_missing_one() {
        // Unlike a rate, a sum over nothing is a real answer: nothing traded.
        assert_eq!(observed_sats(&[], Window::new(0, 1)), 0);
    }
}
