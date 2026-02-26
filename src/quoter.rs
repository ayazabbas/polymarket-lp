use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// A proposed quote with bid and ask prices for a single token side.
#[derive(Debug, Clone)]
pub struct Quote {
    pub bid_price: Decimal,
    pub ask_price: Decimal,
    pub size: Decimal,
    pub level: u32,
}

/// Parameters needed to generate quotes.
#[derive(Debug, Clone)]
pub struct QuoteParams {
    pub midpoint: Decimal,
    pub base_offset_cents: Decimal,
    pub min_offset_cents: Decimal,
    pub tick_size: Decimal,
    pub order_size: Decimal,
    pub num_levels: u32,
    /// Fee rate in basis points (e.g., 200 = 2%). None if no fees.
    pub fee_rate_bps: Option<u32>,
    /// Maximum spread from midpoint that still earns rewards.
    pub max_incentive_spread: Option<Decimal>,
    /// Minimum order size for reward scoring.
    pub min_incentive_size: Option<Decimal>,
    /// Inventory skew: positive = long (widen bid, tighten ask), negative = short
    pub inventory_skew: Decimal,
}

/// Compute the fee-aware offset.
/// For fee-enabled markets: offset = max(min_offset, taker_fee_at_midpoint / 2 + base_spread)
/// The taker fee at midpoint approximation: fee_rate * p * (1-p) where p is midpoint price.
pub fn compute_offset(params: &QuoteParams) -> Decimal {
    let base_offset = params.base_offset_cents / dec!(100); // convert cents to price

    let fee_offset = if let Some(fee_bps) = params.fee_rate_bps {
        let fee_rate = Decimal::new(fee_bps as i64, 4); // bps to decimal
        let p = params.midpoint;
        let fee_at_mid = fee_rate * p * (Decimal::ONE - p);
        fee_at_mid / dec!(2) + base_offset
    } else {
        base_offset
    };

    let min_offset = params.min_offset_cents / dec!(100);
    fee_offset.max(min_offset)
}

/// Align a price to the market's tick size (round to nearest tick).
pub fn align_to_tick(price: Decimal, tick_size: Decimal) -> Decimal {
    if tick_size.is_zero() {
        return price;
    }
    (price / tick_size).round() * tick_size
}

/// Generate quotes for a given set of parameters.
/// Returns quotes for each level on both sides.
pub fn generate_quotes(params: &QuoteParams) -> Vec<Quote> {
    let base_offset = compute_offset(params);
    let mut quotes = Vec::new();

    for level in 0..params.num_levels {
        let level_offset = base_offset + base_offset * Decimal::new(level as i64, 1); // each level 10% wider

        // Apply inventory skew: if long, widen bid (less aggressive buying), tighten ask
        let skew = params.inventory_skew;
        let bid_offset = level_offset * (Decimal::ONE + skew);
        let ask_offset = level_offset * (Decimal::ONE - skew);

        let raw_bid = params.midpoint - bid_offset;
        let raw_ask = params.midpoint + ask_offset;

        let bid_price = align_to_tick(raw_bid, params.tick_size);
        let ask_price = align_to_tick(raw_ask, params.tick_size);

        // Validate price bounds
        if bid_price <= Decimal::ZERO || ask_price >= Decimal::ONE {
            continue;
        }
        if bid_price >= ask_price {
            continue;
        }

        quotes.push(Quote {
            bid_price,
            ask_price,
            size: params.order_size,
            level,
        });
    }

    quotes
}

/// Calculate the quadratic incentive score for a quote.
/// S(v, s) = ((v - s) / v)^2 * b
/// where v = max_incentive_spread, s = distance from midpoint, b = order_size
pub fn estimate_score(
    midpoint: Decimal,
    price: Decimal,
    size: Decimal,
    max_spread: Option<Decimal>,
    min_size: Option<Decimal>,
) -> Decimal {
    let distance = (midpoint - price).abs();

    // Check minimum size
    if let Some(min_sz) = min_size {
        if size < min_sz {
            return Decimal::ZERO;
        }
    }

    let v = match max_spread {
        Some(spread) => {
            if distance > spread {
                return Decimal::ZERO;
            }
            spread
        }
        None => dec!(0.05), // default 5 cent spread assumption
    };

    if v.is_zero() {
        return Decimal::ZERO;
    }

    let ratio = (v - distance) / v;
    ratio * ratio * size
}

/// Calculate the two-sided bonus.
/// Q_min = min(Q_bid, Q_ask). Single-sided orders get divided by 3.
pub fn two_sided_score(bid_score: Decimal, ask_score: Decimal) -> Decimal {
    let q_min = bid_score.min(ask_score);
    let q_max = bid_score.max(ask_score);
    // Two-sided: Q_min counts fully, surplus single-sided divided by 3
    q_min + (q_max - q_min) / dec!(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_offset_no_fee() {
        let params = QuoteParams {
            midpoint: dec!(0.50),
            base_offset_cents: dec!(1.0),
            min_offset_cents: dec!(0.5),
            tick_size: dec!(0.01),
            order_size: dec!(500),
            num_levels: 2,
            fee_rate_bps: None,
            max_incentive_spread: None,
            min_incentive_size: None,
            inventory_skew: Decimal::ZERO,
        };
        let offset = compute_offset(&params);
        assert_eq!(offset, dec!(0.01)); // 1.0 cents = 0.01
    }

    #[test]
    fn test_compute_offset_with_fee() {
        let params = QuoteParams {
            midpoint: dec!(0.50),
            base_offset_cents: dec!(1.0),
            min_offset_cents: dec!(0.5),
            tick_size: dec!(0.01),
            order_size: dec!(500),
            num_levels: 2,
            fee_rate_bps: Some(200), // 2%
            max_incentive_spread: None,
            min_incentive_size: None,
            inventory_skew: Decimal::ZERO,
        };
        let offset = compute_offset(&params);
        // fee_at_mid = 0.02 * 0.50 * 0.50 = 0.005
        // offset = 0.005/2 + 0.01 = 0.0125
        assert_eq!(offset, dec!(0.0125));
    }

    #[test]
    fn test_align_to_tick() {
        assert_eq!(align_to_tick(dec!(0.4567), dec!(0.01)), dec!(0.46));
        assert_eq!(align_to_tick(dec!(0.4567), dec!(0.001)), dec!(0.457));
        assert_eq!(align_to_tick(dec!(0.4567), dec!(0.1)), dec!(0.5));
        assert_eq!(align_to_tick(dec!(0.4567), dec!(0.0001)), dec!(0.4567));
    }

    #[test]
    fn test_generate_quotes_basic() {
        let params = QuoteParams {
            midpoint: dec!(0.50),
            base_offset_cents: dec!(1.0),
            min_offset_cents: dec!(0.5),
            tick_size: dec!(0.01),
            order_size: dec!(500),
            num_levels: 2,
            fee_rate_bps: None,
            max_incentive_spread: None,
            min_incentive_size: None,
            inventory_skew: Decimal::ZERO,
        };
        let quotes = generate_quotes(&params);
        assert_eq!(quotes.len(), 2);
        // Level 0: bid=0.49, ask=0.51
        assert_eq!(quotes[0].bid_price, dec!(0.49));
        assert_eq!(quotes[0].ask_price, dec!(0.51));
    }

    #[test]
    fn test_estimate_score() {
        let score = estimate_score(
            dec!(0.50),
            dec!(0.49), // 1 cent away
            dec!(1000),
            Some(dec!(0.05)), // 5 cent max spread
            None,
        );
        // ((0.05 - 0.01) / 0.05)^2 * 1000 = (0.8)^2 * 1000 = 640
        assert_eq!(score, dec!(640));
    }

    #[test]
    fn test_estimate_score_outside_spread() {
        let score = estimate_score(
            dec!(0.50),
            dec!(0.40), // 10 cents away, > max 5 cent spread
            dec!(1000),
            Some(dec!(0.05)),
            None,
        );
        assert_eq!(score, Decimal::ZERO);
    }

    #[test]
    fn test_estimate_score_below_min_size() {
        let score = estimate_score(
            dec!(0.50),
            dec!(0.49),
            dec!(10), // below min 50
            Some(dec!(0.05)),
            Some(dec!(50)),
        );
        assert_eq!(score, Decimal::ZERO);
    }

    #[test]
    fn test_two_sided_score() {
        // Balanced: both sides score 640
        assert_eq!(two_sided_score(dec!(640), dec!(640)), dec!(640));
        // Imbalanced: bid=640, ask=100
        // Q_min=100, surplus=540/3=180, total=280
        assert_eq!(two_sided_score(dec!(640), dec!(100)), dec!(280));
    }
}
