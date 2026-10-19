use std::path::Path;

use crate::syntax::test_syntect;
use crate::{MarkdownStyle, StreamingMarkdownRenderer, render_markdown_ratatui_full};

const CASES: &[(&str, &str)] = &[
    (
        "js",
        "const café = `hello ${'日本語'}`;\nconst node = <div>{café}</div>;\n",
    ),
    (
        "jsx",
        "export const App = () => <main title=\"😀\">Hello</main>;\n",
    ),
    (
        "tsx",
        "/* comment\ncontinued */\nexport const App = (p: {name: string}) => <div>{p.name}</div>;\n",
    ),
    (
        "html",
        "<script>const value = /a+b/giu;\n</script><style>body { color: red; }</style>\n",
    ),
    (
        "sh",
        "cat <<'EOF'\nhello 日本語\nEOF\nprintf '%s' \"done\"\n",
    ),
    ("ps1", "$name = \"café\"\nWrite-Output $name\n"),
    (
        "py",
        "value = \"\"\"multiline\n日本語\"\"\"\nprint(value)\n",
    ),
    (
        "rs",
        "fn main() { let value = \"naïve 😀\"; println!(\"{value}\"); }\n",
    ),
    ("cpp", "template<class T> T answer(T x) { return x + 1; }\n"),
    ("yaml", "name: café\nmessage: |\n  日本語\n  😀\n"),
    ("dsl", "Invoice\n  id BigInt\n  status Status\n"),
    ("res", "type media = Image | Video\nlet label = \"café\"\n"),
    ("resi", "type media = Image | Video\n"),
    (
        "swift",
        "let s = \"\"\"\n{\"name\": \"buddy\"}\\(x)\n\"\"\"\n// MARK: after\n",
    ),
];

#[test]
fn syntax_backend_preserves_source_bytes() {
    let syntect = test_syntect();
    for &(extension, source) in CASES {
        for (variant, text) in [
            ("LF", source.to_owned()),
            ("CRLF", source.replace('\n', "\r\n")),
            ("trimmed", source.trim_end().to_owned()),
        ] {
            let path = format!("example.{extension}");
            let mut highlighter = syntect
                .highlight_lines_by_file_path(Path::new(&path))
                .unwrap();
            let mut reconstructed = String::new();
            for line in syntect::util::LinesWithEndings::from(&text) {
                for (_, segment) in highlighter
                    .highlight_line(line, &syntect.syntax_set)
                    .unwrap()
                {
                    reconstructed.push_str(segment);
                }
            }
            assert_eq!(text, reconstructed, "{extension}, variant={variant}");
        }
    }
}

#[test]
fn syntax_backend_streaming_matches_batch_for_open_and_closed_fences() {
    let syntect = test_syntect();
    for &(extension, source) in CASES {
        for (fence, ending) in [("open", ""), ("closed", "```\n\nDone.\n")] {
            let text = format!("```{extension}\n{source}{ending}");
            let expected =
                render_markdown_ratatui_full(&text, MarkdownStyle::default(), true, Some(syntect))
                    .0;
            for chunk_chars in [1, 7, 31] {
                let mut renderer = StreamingMarkdownRenderer::new(MarkdownStyle::default(), true);
                let mut chunk = String::new();
                for (index, character) in text.chars().enumerate() {
                    chunk.push(character);
                    if (index + 1) % chunk_chars == 0 {
                        renderer.push_and_render(&chunk, Some(syntect));
                        chunk.clear();
                    }
                }
                renderer.push_and_render(&chunk, Some(syntect));
                assert_eq!(
                    expected.lines.as_slice(),
                    renderer.view().lines,
                    "streamed {extension}, fence={fence}, chunk={chunk_chars}"
                );
                let actual = renderer.finish(Some(syntect));
                assert_eq!(
                    expected.lines.as_slice(),
                    actual.lines,
                    "{extension}, fence={fence}, chunk={chunk_chars}"
                );
            }
        }
    }
}

#[test]
fn resolves_extension_to_expected_grammar() {
    let syntect = test_syntect();
    for (extension, name) in [
        ("js", "JavaScript (Babel)"),
        ("jsx", "JavaScript (Babel)"),
        ("ps1", "PowerShell"),
        ("dsl", "DSL"),
        ("res", "ReScript"),
        ("resi", "ReScript"),
        ("re", "ReScript"),
        ("rei", "ReScript"),
        ("fs", "F#"),
        ("ml", "OCaml"),
    ] {
        assert_eq!(
            name,
            syntect
                .find_syntax_by_file_path(Path::new(&format!("example.{extension}")))
                .unwrap()
                .name
        );
    }
}

#[test]
fn unknown_and_empty_fences_use_dsl_while_known_fences_keep_their_grammar() {
    let syntect = test_syntect();
    for (fence, expected) in [
        ("dsl", "DSL"),
        ("", "DSL"),
        ("javasript", "DSL"),
        ("37:65:src/unknown.extension", "DSL"),
        ("rescript", "ReScript"),
        ("reason", "ReScript"),
        ("ReasonML", "ReScript"),
        ("fsharp", "F#"),
        ("F#", "F#"),
        ("ocaml", "OCaml"),
        ("rust", "Rust"),
    ] {
        assert_eq!(
            syntect
                .find_syntax_for_fence_info(fence)
                .map(|s| s.name.as_str()),
            Some(expected),
            "fence: {fence}"
        );
    }

    for fence in ["", "javasript", "rescript"] {
        let source = format!("```{fence}\nlet media = Image\n```\n");
        let expected =
            render_markdown_ratatui_full(&source, MarkdownStyle::default(), true, Some(syntect)).0;
        let mut renderer = StreamingMarkdownRenderer::new(MarkdownStyle::default(), true);
        renderer.push_and_render(&source, Some(syntect));
        assert_eq!(
            expected.lines.as_slice(),
            renderer.finish(Some(syntect)).lines
        );
    }
}
