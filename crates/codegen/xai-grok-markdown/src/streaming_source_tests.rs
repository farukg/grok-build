use super::*;
use crate::style::test_style;

fn renderer(pretty: bool) -> StreamingMarkdownRenderer {
    let mut renderer = StreamingMarkdownRenderer::new(test_style::STYLE, pretty);
    renderer.set_tab_width(4);
    renderer
}

#[test]
fn original_source_includes_tabs_unicode_and_pending_delimiters() {
    let chunks = [
        " \t\n",
        "界e\u{301}",
        "\\",
        "(",
        " x ",
        "\\)",
        "\n\n```",
        "text\n\t界\n",
        "```",
    ];
    let mut markdown = renderer(true);
    let mut expected = String::new();
    for chunk in chunks {
        expected.push_str(chunk);
        markdown.push_and_render(chunk, None);
        assert_eq!(markdown.source().as_bytes(), expected.as_bytes());
        let cloned = markdown.clone();
        assert_eq!(cloned.source().as_bytes(), expected.as_bytes());
        assert_eq!(cloned.view().lines, markdown.view().lines);
    }
    markdown.finish(None);
    assert_eq!(markdown.source().as_bytes(), expected.as_bytes());
    assert_eq!(markdown.frozen_bytes(), expected.len());
    markdown.clear();
    assert_eq!(markdown.source(), "");
    markdown.push_and_render("é\tZ", None);
    assert_eq!(markdown.source(), "é\tZ");
}

#[test]
fn tab_projection_is_chunk_invariant_in_pretty_and_raw_modes() {
    let source = "界\t[é](https://x.test)\n\n```text\n\t界\té\n```\n";
    let display_source = "界    [é](https://x.test)\n\n```text\n    界    é\n```\n";
    for pretty in [true, false] {
        let (expected, _) =
            render_markdown_ratatui_full(display_source, test_style::STYLE, pretty, None);
        for split in source
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(source.len()))
        {
            let mut markdown = renderer(pretty);
            markdown.push_and_render(source.get(..split).unwrap(), None);
            markdown.push_and_render(source.get(split..).unwrap(), None);
            markdown.finish(None);
            assert_eq!(markdown.source(), source, "split {split}");
            assert_eq!(
                markdown.view().lines,
                expected.lines.as_slice(),
                "split {split}, pretty {pretty}"
            );
            assert_eq!(markdown.view().hyperlinks, expected.hyperlinks.as_slice());
            assert_eq!(
                markdown.view().line_source_map,
                expected.line_source_map.as_slice()
            );
        }
    }
}

#[test]
fn zero_and_configured_tab_widths_keep_original_code_ranges() {
    let source = "# H\n\n```text\né\t界\n```\n";
    let start = source.find('é').unwrap();
    for (width, body) in [
        (0, "é\t界\n"),
        (1, "é 界\n"),
        (2, "é  界\n"),
        (8, "é        界\n"),
    ] {
        for pretty in [true, false] {
            let mut markdown = StreamingMarkdownRenderer::new(test_style::STYLE, pretty);
            markdown.set_tab_width(width);
            for ch in source.chars() {
                markdown.push_and_render(&ch.to_string(), None);
            }
            markdown.finish(None);
            assert_eq!(markdown.source(), source);
            assert_eq!(markdown.frozen_bytes(), source.len());
            let block = markdown.view().code_blocks.first().unwrap();
            assert_eq!(block.body, body);
            assert_eq!(block.source_byte_range, start..start + "é\t界\n".len());
            assert_eq!(
                markdown.source().get(block.source_byte_range.clone()),
                Some("é\t界\n")
            );
        }
    }
}

#[test]
fn tab_link_targets_use_display_cells_before_and_after_finish() {
    for (pretty, label_columns, url_columns) in [(true, 6..7, 9..23), (false, 7..8, 10..24)] {
        let mut markdown = renderer(pretty);
        for chunk in ["界", "\t[", "é", "](https://x.test)"] {
            markdown.push_and_render(chunk, None);
        }
        for finished in [false, true] {
            if finished {
                markdown.finish(None);
            }
            let links = markdown.view().hyperlinks;
            assert_eq!(links.len(), 2);
            let label = links.first().unwrap();
            let url = links.get(1).unwrap();
            assert_eq!(label.column_range, label_columns);
            assert_eq!(url.column_range, url_columns);
            assert!(links.iter().all(|link| link.url == "https://x.test"));
            assert_ne!(label.id, url.id);
        }
    }
}

#[test]
fn code_ranges_and_frozen_offsets_address_original_bytes() {
    let source = "# H\n\n\\( x \\)\n\n```text\n界\té\n```\n\n";
    let body_start = source.find("界").unwrap();
    let body_end = body_start + "界\té\n".len();
    let mut markdown = renderer(true);
    for ch in source.chars() {
        markdown.push_and_render(&ch.to_string(), None);
    }
    for finished in [false, true] {
        if finished {
            markdown.finish(None);
        }
        let blocks = markdown.view().code_blocks;
        assert_eq!(blocks.len(), 1);
        let block = blocks.first().unwrap();
        assert_eq!(block.source_byte_range, body_start..body_end);
        assert_eq!(
            markdown.source().get(block.source_byte_range.clone()),
            Some("界\té\n")
        );
        assert_eq!(block.body, "界    é\n");
        assert!(markdown.source().is_char_boundary(markdown.frozen_bytes()));
    }
    assert_eq!(markdown.frozen_bytes(), source.len());
    let (batch, _) = render_markdown_ratatui_full(source, test_style::STYLE, true, None);
    assert_eq!(
        batch.code_blocks.first().unwrap().source_byte_range,
        body_start..body_end
    );
}

#[test]
fn completed_raw_pretty_roundtrip_keeps_source_and_closed_code() {
    let source = "\\( x \\)\n\n```text\n\t界\n```";
    let mut markdown = renderer(true);
    markdown.push(source);
    markdown.finish(None);
    let pretty_lines = markdown.view().lines.to_vec();
    markdown.set_pretty(false);
    markdown.render(None);
    assert_eq!(markdown.source(), source);
    assert!(
        markdown
            .view()
            .lines
            .iter()
            .any(|line| line.to_string().contains("\\( x \\)"))
    );
    markdown.set_pretty(true);
    markdown.render(None);
    assert_eq!(markdown.view().lines, pretty_lines.as_slice());
    assert_eq!(markdown.source(), source);
    assert!(markdown.set_tab_width(2));
    markdown.render(None);
    assert_eq!(markdown.source(), source);
    assert_eq!(markdown.view().code_blocks.first().unwrap().body, "  界\n");
}

#[test]
fn source_line_maps_follow_original_lines_across_frozen_and_normalized_blocks() {
    let source = "# Start\n\n\\[\nx\n\\]\n\n# End\n";
    let mut markdown = renderer(true);
    for chunk in ["# Start\n\n", "\\[\nx\n", "\\]\n\n", "# End\n"] {
        markdown.push_and_render(chunk, None);
    }
    for finished in [false, true] {
        if finished {
            markdown.finish(None);
        }
        let view = markdown.view();
        let end = view
            .lines
            .iter()
            .position(|line| line.to_string().contains("End"))
            .unwrap();
        assert_eq!(view.line_source_map.get(end), Some(&6));
    }
    let (batch, _) = render_markdown_ratatui_full(source, test_style::STYLE, true, None);
    let end = batch
        .lines
        .iter()
        .position(|line| line.to_string().contains("End"))
        .unwrap();
    assert_eq!(batch.line_source_map.get(end), Some(&6));
    markdown.set_pretty(false);
    markdown.render(None);
    let view = markdown.view();
    let end = view
        .lines
        .iter()
        .position(|line| line.to_string().contains("End"))
        .unwrap();
    assert_eq!(view.line_source_map.get(end), Some(&6));
    assert_eq!(markdown.source(), source);
}

#[test]
fn streamed_code_lines_keep_original_indices_after_multiline_math() {
    let mut markdown = renderer(true);
    for chunk in ["# Start\n\n", "\\[\nx\n", "\\]\n\n", "```text\n"] {
        markdown.push_and_render(chunk, None);
    }
    for count in 1..=32 {
        markdown.push_and_render("界\té\n", None);
        let expected: Vec<usize> = (7..7 + count).collect();
        for _ in 0..2 {
            let view = markdown.view();
            let mapped: Vec<usize> = view
                .lines
                .iter()
                .zip(view.line_source_map)
                .filter_map(|(line, &source)| (line.to_string() == "界    é").then_some(source))
                .collect();
            assert_eq!(mapped, expected);
            markdown.render(None);
        }
    }
    markdown.push_and_render("```\n", None);
    markdown.finish(None);
    let cloned = markdown.clone();
    assert_eq!(cloned.view().line_source_map, markdown.view().line_source_map);
    assert_eq!(cloned.source(), markdown.source());
    markdown.clear();
    markdown.push_and_render("# Fresh\n", None);
    assert_eq!(markdown.view().line_source_map.first(), Some(&0));
}

#[test]
fn original_append_offset_survives_normalizer_holdback_and_clone() {
    let mut markdown = renderer(true);
    markdown.push_and_render("é\t\\", None);
    let offset = markdown.source().len();
    assert_eq!(offset, 4);
    let mut resumed = markdown.clone();
    resumed.push_and_render("( x \\)\t界", None);
    assert_eq!(resumed.source().get(offset..), Some("( x \\)\t界"));
    resumed.finish(None);
    assert_eq!(resumed.source(), "é\t\\( x \\)\t界");
    assert_eq!(markdown.source(), "é\t\\");
}
