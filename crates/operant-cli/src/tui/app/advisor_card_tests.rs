// app/advisor_card_tests.rs — the advisor card the operator READS.
//
// The advisor's notes arrive as ONE BackgroundReview summary string. Two
// different things live in that string: the CARD (the notes, which the
// operator reads) and the CONTINUATION PAYLOAD (the model-visible batch, a
// machine payload). They used to be rendered as one lump — the notes joined
// with `"; "`, and the raw boundary guidance + `<advisory>` XML on top of it
// for a continue-channel review.
//
// The operator's report: "advisor preview does not render correctly. It lumps
// everything without correct formatting, rather than showing it in blocks".

use super::*;
use operant_core::agent::AgentEvent;

fn review(summary: String) -> AgentEvent {
    AgentEvent::BackgroundReview {
        summary,
        session_id: "sess-adv".to_string(),
    }
}

/// Render `app` once and return the screen as one string per row.
fn screen_rows(app: &App) -> Vec<String> {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|f| crate::tui::render::render_app(f, app))
        .unwrap();
    let buf = terminal.backend().buffer();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

/// The card must show ONE BLOCK PER NOTE: each note on its own row, so a
/// three-note review reads as three judgements instead of one run-on
/// sentence.
///
/// MUTATION-PROVEN: joining the notes with `"; "` (the old preserve channel)
/// puts both texts on ONE row and fails the "different rows" assertion.
#[test]
fn the_advisor_card_shows_one_block_per_note() {
    let mut app = super::tests::make_app();
    app.handle_agent_event(review(
        "⚑ advisor (small-stack):\n\n[concern] fix the race\n\n[nit] typo in the comment"
            .to_string(),
    ));

    let rows = screen_rows(&app);
    let concern_row = rows.iter().position(|r| r.contains("fix the race"));
    let nit_row = rows.iter().position(|r| r.contains("typo in the comment"));
    assert!(concern_row.is_some(), "the concern note must be on screen");
    assert!(nit_row.is_some(), "the nit note must be on screen");
    assert_ne!(
        concern_row, nit_row,
        "each note gets its OWN row (a block), never one lumped line: {rows:?}"
    );
}

/// The continuation payload is for the MODEL, not the operator's screen: the
/// boundary guidance and the raw `<advisory>` XML must never be painted in the
/// transcript.
#[test]
fn the_continuation_payload_never_reaches_the_card() {
    let mut app = super::tests::make_app();
    let marker = operant_core::advisor::CONTINUATION_TAG;
    let batch = format!(
        "{marker}\nThe following is an aggregated review from other models.\n<advisory severity=\"concern\">fix the race</advisory>",
    );
    app.handle_agent_event(review(format!(
        "⚑ advisor (small-stack):\n\n[concern] fix the race\n\n{batch}"
    )));

    // The batch is queued for the next turn...
    let queued = app
        .pending_advisor_continuation
        .clone()
        .expect("a continue-channel review queues the batch");
    assert!(
        queued.starts_with(marker),
        "the marker rides the queued batch"
    );
    assert!(
        queued.contains("<advisory"),
        "the model still receives the advisory elements"
    );

    // ...and the card shows the NOTE, not the payload.
    let rows = screen_rows(&app);
    assert!(
        rows.iter().any(|r| r.contains("fix the race")),
        "the note is on screen"
    );
    assert!(
        !rows.iter().any(|r| r.contains("<advisory")),
        "the raw advisory XML must never be painted in the transcript: {rows:?}"
    );
    assert!(
        !rows.iter().any(|r| r.contains(marker)),
        "the continuation marker is machine payload, not card text: {rows:?}"
    );
}
