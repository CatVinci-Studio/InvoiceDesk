//! Cross-field checks, run after every extraction path has had its turn.
//!
//! These are the app's substitute for 发票查验: it does not call the tax
//! authority, so it cannot prove an invoice is genuine - but it CAN prove
//! that a set of numbers is internally inconsistent, and that catches the
//! failure that actually happens, which is a misread digit rather than a
//! forgery.
//!
//! The 价税合计 = 金额 + 税额 check earns its place several times over:
//! a single wrong digit in ANY of the three makes the identity fail, so one
//! comparison covers the three fields that matter most. Nothing here ever
//! rejects an invoice; problems are recorded on it so the review UI can show
//! them and the user can fix them.

use crate::model::{Invoice, InvoiceKind, Money, ValidationIssue};
use chrono::NaiveDate;

/// Today, for the future-date check. Injected rather than read from the
/// clock so the tests are not time bombs.
pub struct Clock {
    pub today: NaiveDate,
}

impl Default for Clock {
    fn default() -> Self {
        Clock {
            today: chrono::Local::now().date_naive(),
        }
    }
}

/// Fills in whatever the extraction layers left implicit.
///
/// Runs BEFORE the checks, on purpose: an invoice whose 价税合计 was never
/// printed but whose 金额 and 税额 both were is complete, not defective, and
/// reporting "缺少价税合计" on it would be noise. The derived field is marked
/// down in confidence and inherits the weaker parent's source, so it still
/// shows up for review if either input was shaky.
pub fn infer_missing(invoice: &mut Invoice) {
    let (amount, tax, total) = (
        invoice.amount_excl_tax.value,
        invoice.tax.value,
        invoice.total.value,
    );

    // A derived value is never better than its worst input, and slightly
    // worse than it - it is arithmetic on a reading, not a reading.
    let derived = |a: &crate::model::Field<Money>, b: &crate::model::Field<Money>, value: Money| {
        crate::model::Field::with_confidence(
            value,
            a.source.min(b.source),
            a.confidence.min(b.confidence) * 0.95,
        )
    };

    match (amount, tax, total) {
        (Some(a), Some(t), None) => {
            invoice.total = derived(&invoice.amount_excl_tax, &invoice.tax, a + t);
        }
        (Some(a), None, Some(tot)) => {
            invoice.tax = derived(&invoice.amount_excl_tax, &invoice.total, tot - a);
        }
        (None, Some(t), Some(tot)) => {
            invoice.amount_excl_tax = derived(&invoice.tax, &invoice.total, tot - t);
        }
        _ => {}
    }
}

/// How many digits this 票种's 发票号码 must have.
fn expected_number_length(kind: Option<InvoiceKind>) -> Option<usize> {
    match kind {
        Some(k) if k.is_fully_digital() => Some(20),
        Some(
            InvoiceKind::VatSpecial
            | InvoiceKind::VatGeneral
            | InvoiceKind::VatElectronicGeneral
            | InvoiceKind::VatRoll,
        ) => Some(8),
        // Train tickets, itineraries and fixed-amount receipts carry
        // numbers of their own shape; there is nothing to check against.
        _ => None,
    }
}

/// Every problem found, worst first.
pub fn check(invoice: &Invoice, clock: &Clock) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // --- the identity that catches misread digits ---
    if let (Some(amount), Some(tax), Some(total)) = (
        invoice.amount_excl_tax.value,
        invoice.tax.value,
        invoice.total.value,
    ) {
        // Exact, with no tolerance: an invoice's own 合计 row is computed by
        // the issuing system, so any difference at all is a reading error on
        // this side, not a rounding artefact on theirs.
        if amount + tax != total {
            issues.push(ValidationIssue::TotalMismatch {
                expected: amount + tax,
                found: total,
            });
        }
    }

    // --- required fields ---
    for (present, name) in [
        (invoice.number.is_present(), "发票号码"),
        (invoice.issued_on.is_present(), "开票日期"),
        (invoice.total.is_present(), "价税合计"),
    ] {
        if !present {
            issues.push(ValidationIssue::MissingField {
                field: name.to_string(),
            });
        }
    }

    // --- number shape ---
    if let (Some(number), Some(expected)) = (
        invoice.number.as_ref(),
        expected_number_length(invoice.kind),
    ) {
        if number.len() != expected {
            issues.push(ValidationIssue::NumberLength {
                found: number.len(),
                expected,
            });
        }
    }

    // --- date sanity ---
    if let Some(raw) = invoice.issued_on.as_ref() {
        match NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
            Ok(date) if date > clock.today => {
                issues.push(ValidationIssue::FutureDate { date: raw.clone() });
            }
            Ok(_) => {}
            Err(_) => issues.push(ValidationIssue::BadDate { raw: raw.clone() }),
        }
    }

    issues.sort_by_key(|i| !i.is_error());
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Field, FieldSource};

    fn clock() -> Clock {
        Clock {
            today: NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(),
        }
    }

    fn invoice(amount: i64, tax: i64, total: i64) -> Invoice {
        Invoice {
            kind: Some(InvoiceKind::DigitalGeneral),
            number: Field::new("24312000000012345678".to_string(), FieldSource::Xml),
            issued_on: Field::new("2024-03-01".to_string(), FieldSource::Xml),
            amount_excl_tax: Field::new(Money(amount), FieldSource::Xml),
            tax: Field::new(Money(tax), FieldSource::Xml),
            total: Field::new(Money(total), FieldSource::Xml),
            ..Default::default()
        }
    }

    #[test]
    fn a_consistent_invoice_has_no_issues() {
        assert!(check(&invoice(100_000, 6_000, 106_000), &clock()).is_empty());
    }

    /// One wrong digit anywhere in the three amounts trips this.
    #[test]
    fn a_misread_digit_breaks_the_identity() {
        let issues = check(&invoice(100_000, 6_000, 105_000), &clock());
        assert_eq!(
            issues[0],
            ValidationIssue::TotalMismatch {
                expected: Money(106_000),
                found: Money(105_000),
            }
        );
        assert!(issues[0].is_error());
    }

    #[test]
    fn digital_invoices_must_carry_twenty_digit_numbers() {
        let mut inv = invoice(100_000, 6_000, 106_000);
        inv.number = Field::new("12345678".to_string(), FieldSource::PdfText);
        let issues = check(&inv, &clock());
        assert!(issues.contains(&ValidationIssue::NumberLength {
            found: 8,
            expected: 20
        }));
    }

    /// Train tickets and itineraries have their own numbering; checking them
    /// against the VAT shapes would flag every one of them.
    #[test]
    fn transport_receipts_are_not_checked_for_vat_number_shape() {
        let mut inv = invoice(100_000, 6_000, 106_000);
        inv.kind = Some(InvoiceKind::Train);
        inv.number = Field::new("E123456789".to_string(), FieldSource::PdfText);
        assert!(check(&inv, &clock())
            .iter()
            .all(|i| !matches!(i, ValidationIssue::NumberLength { .. })));
    }

    #[test]
    fn a_future_date_is_flagged() {
        let mut inv = invoice(100_000, 6_000, 106_000);
        inv.issued_on = Field::new("2025-01-01".to_string(), FieldSource::PdfText);
        assert!(
            check(&inv, &clock()).contains(&ValidationIssue::FutureDate {
                date: "2025-01-01".to_string()
            })
        );
    }

    #[test]
    fn missing_essentials_are_reported_by_name() {
        let issues = check(&Invoice::default(), &clock());
        for field in ["发票号码", "开票日期", "价税合计"] {
            assert!(
                issues.contains(&ValidationIssue::MissingField {
                    field: field.to_string()
                }),
                "{field} 未被报告"
            );
        }
    }

    #[test]
    fn the_missing_third_amount_is_derived_rather_than_reported() {
        let mut inv = invoice(100_000, 6_000, 0);
        inv.total = Field::default();
        infer_missing(&mut inv);
        assert_eq!(inv.total.value, Some(Money(106_000)));
        assert!(check(&inv, &clock()).is_empty());
    }

    /// A derived value must be marked down, or a shaky reading launders
    /// itself into a confident-looking total.
    #[test]
    fn derived_amounts_inherit_the_weaker_parent() {
        let mut inv = invoice(100_000, 6_000, 0);
        inv.total = Field::default();
        inv.amount_excl_tax = Field::new(Money(100_000), FieldSource::Vision);
        infer_missing(&mut inv);
        assert_eq!(inv.total.source, FieldSource::Vision);
        assert!(inv.total.confidence < FieldSource::Vision.base_confidence());
        assert!(inv.total.needs_review());
    }

    #[test]
    fn errors_sort_ahead_of_cautions() {
        let mut inv = invoice(100_000, 6_000, 105_000);
        inv.number = Field::new("12345678".to_string(), FieldSource::PdfText);
        let issues = check(&inv, &clock());
        assert!(issues[0].is_error(), "错误应排在提醒之前");
    }
}
