// Vendored from jcode (crates/jcode-tui-markdown/src/markdown_tests/cases/latex_streaming.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805,
// no changes (cfg(feature = "mermaid-renderer") tests kept; feature absent in
// operant, so those compile out). See jcode_markdown/mod.rs for scope.

fn exact_multiline_latex_response() -> &'static str {
    concat!(
        "\\[\n\\boxed{\ne^{i\\pi}+1=0\n}\n\\]\n\n",
        "\\[\n\\int_{-\\infty}^{\\infty} e^{-x^2}\\,dx=\\sqrt{\\pi}\n\\]\n\n",
        "\\[\nx=\\frac{-b\\pm\\sqrt{b^2-4ac}}{2a}\n\\]\n\n",
        "\\[\n\\nabla\\cdot\\mathbf{E}=\\frac{\\rho}{\\varepsilon_0}\n\\]\n\n",
        "\\[\n\\frac{\\partial \\psi}{\\partial t}\n=\n",
        "\\alpha\\frac{\\partial^2\\psi}{\\partial x^2}\n\\]",
    )
}

#[test]
fn latex_foreground_is_white_and_styles_inline_math() {
    assert_eq!(MATH_FOREGROUND, (255, 255, 255));
    assert_eq!(MATH_INLINE_FOREGROUND, (255, 255, 255));

    let lines = with_streaming_render_context(|| render_markdown("Inline $x^2$ math."));
    let math_spans: Vec<_> = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .filter(|span| span.content.contains("x²"))
        .collect();
    assert_eq!(math_spans.len(), 1);
    assert_eq!(math_spans[0].style.fg, Some(crate::tui::jcode_markdown::math_inline_fg()));
}

#[test]
fn exact_multiline_response_renders_all_five_equations() {
    let mut renderer = IncrementalMarkdownRenderer::new(Some(90));
    let rendered = lines_to_string(&renderer.update(exact_multiline_latex_response()));

    assert_eq!(rendered.matches("┌─ math").count(), 5, "{rendered}");
    assert!(!rendered.contains("$$"), "{rendered}");
    assert!(!rendered.contains(r"\partial"), "{rendered}");
    assert!(rendered.contains('∂'), "{rendered}");
    assert!(rendered.contains('α'), "{rendered}");
}

#[test]
fn every_streaming_prefix_converges_to_the_full_math_render() {
    let response = exact_multiline_latex_response();
    let mut renderer = IncrementalMarkdownRenderer::new(Some(90));

    for end in response
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(response.len()))
    {
        let _ = renderer.update(&response[..end]);
    }

    let incremental = renderer.update(response);
    let full = with_streaming_render_context(|| render_markdown_with_width(response, Some(90)));
    assert_eq!(incremental, full);
    assert_eq!(lines_to_string(&incremental).matches("┌─ math").count(), 5);
}

// [port-excision] Three tests that exercise the latex-image machinery's test
// helpers (test_toolchain_runs / reset_test_render_attempts /
// test_render_attempts) are removed with markdown_latex_image.rs (not ported;
// mdwright-latex dep absent - see latex_image_lines port-decision in mod.rs):
// - math_rendering_never_runs_the_toolchain_on_the_render_thread
// - inline_math_stays_inline_in_image_mode_and_skips_the_image_toolchain
// - display_math_nested_in_a_list_uses_graphical_rendering
// They assert on image-toolchain call counts, which cannot run here. The
// unicode-path tests in this file (streaming convergence, multiline
// equations, blockquote math, promoted delimiters) are ported and green.

#[test]
fn multiline_relations_survive_blockquotes_and_promoted_delimiters() {
    let source = concat!(
        "> Blockquote display:\n> \\[\n> x^2\n> =\n> y^2\n> \\]\n\n",
        "Standalone spelling:\n\\(\nx\n=\ny\n\\)",
    );
    let rendered = with_streaming_render_context(|| {
        lines_to_string(&render_markdown_with_width(source, Some(90)))
    });

    assert_eq!(rendered.matches("┌─ math").count(), 2, "{rendered}");
    assert!(rendered.contains("x² = y²"), "{rendered}");
    assert!(rendered.contains("x = y"), "{rendered}");
    assert!(!rendered.contains("{}="), "{rendered}");
    assert!(!rendered.contains("$$"), "{rendered}");
}
