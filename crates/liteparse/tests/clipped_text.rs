//! Synthetic PDFs keep clipping regressions independent of customer documents.
use liteparse::{RawTextItem, extract_raw_text_items, stages};
use pdfium::Library;

fn pdf(content: &str, forms: &[&str], rotation: i32) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Rotate {rotation} /Resources << /Font << /F1 4 0 R >> /XObject << /Outer 6 0 R >> >> /Contents 5 0 R >>"
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_string(),
        stream("", content),
    ];
    objects.extend(forms.iter().map(|s| s.to_string()));
    let mut data = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(data.len());
        data.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = data.len();
    data.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        data.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    data.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    data
}

fn stream(dict: &str, content: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
}

fn extract(bytes: &[u8]) -> (Vec<RawTextItem>, String) {
    let lib = Library::init();
    let doc = lib.load_document_from_bytes(bytes, None).unwrap();
    let page = doc.page(0).unwrap();
    let text = page.text().unwrap();
    let raw = extract_raw_text_items(&page, &text, &page.view_box().unwrap(), None);
    let standard = stages::extract(&doc, &stages::ExtractRequest::default()).unwrap();
    let standard = standard.pages[0]
        .text_items
        .iter()
        .map(|i| i.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    (raw, standard)
}

fn assert_text(bytes: &[u8], expected: &str) {
    let (raw, standard) = extract(bytes);
    let raw: String = raw.iter().map(|i| i.text.as_str()).collect();
    // PDFium inserts separators between source objects. Compare real characters.
    let compact = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    assert_eq!(compact(&raw), compact(expected), "raw: {raw:?}");
    assert_eq!(
        compact(&standard),
        compact(expected),
        "standard: {standard:?}"
    );
}

#[test]
fn clipped_overflow_does_not_leak_into_next_row() {
    let content = "q 100 200 70 8 re W* n BT /F1 6 Tf 105 201 Td (VISIBLE) Tj 0 -7 Td (42) Tj ET Q BT /F1 6 Tf 105 194 Td (NEXT) Tj ET";
    assert_text(&pdf(content, &[], 0), "VISIBLE NEXT");
}

#[test]
fn horizontal_partial_glyphs_survive_in_full_at_every_page_rotation() {
    for rotation in [0, 90, 180, 270] {
        for (edge, expected) in [(26, "ABC"), (24, "ABC"), (22, "ABC"), (21, "AB")] {
            let content = format!(
                "q 10 10 {} 30 re W n BT /F1 10 Tf 10 20 Td (ABCDE) Tj ET Q",
                edge - 10
            );
            assert_text(&pdf(&content, &[], rotation), expected);
        }
    }
}

#[test]
fn nested_forms_compose_transforms_and_inherit_outer_clips() {
    let inner = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /Font << /F1 4 0 R >> >>",
        "0 0 25 80 re W n BT /F1 10 Tf 5 20 Td (ABCDE) Tj ET",
    );
    let outer = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 200 200] /Resources << /XObject << /Inner 7 0 R >> >>",
        "2 0 0 2 10 15 cm /Inner Do",
    );
    let content = "q 100 100 35 300 re W n 1 0 0 1 100 100 cm /Outer Do Q";
    assert_text(&pdf(content, &[&outer, &inner], 0), "AB");
}

#[test]
fn form_bbox_clips_text_without_explicit_clip_operator() {
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 17 100] /Resources << /Font << /F1 4 0 R >> >>",
        "BT /F1 10 Tf 0 20 Td (ABCDE) Tj ET",
    );
    assert_text(&pdf("1 0 0 1 100 100 cm /Outer Do", &[&form], 0), "ABC");
}

#[test]
fn stacked_clips_intersect_and_restore_does_not_leak() {
    let content = "q 0 0 100 100 re W n 10 10 14 30 re W* n BT /F1 10 Tf 10 20 Td (ABCDE) Tj ET Q BT /F1 10 Tf 100 100 Td (RESTORED) Tj ET";
    assert_text(&pdf(content, &[], 0), "ABC RESTORED");
}

#[test]
fn unsupported_clips_preserve_source_text() {
    for path in [
        "0 0 m 1 0 l 1 1 l h",   // triangle, all text outside
        "0 0 1 1 re 2 2 1 1 re", // compound even-odd path
        "0 0 m 1 0 1 1 0 1 c h", // curve
    ] {
        let content = format!("q {path} W* n BT /F1 10 Tf 10 20 Td (KEEP) Tj ET Q");
        assert_text(&pdf(&content, &[], 0), "KEEP");
    }
}

#[test]
fn unclipped_raw_items_are_identical_with_redundant_clip() {
    let text = "BT /F1 10 Tf 10 20 Td (UNCHANGED WORDS) Tj ET";
    let plain = extract(&pdf(text, &[], 0));
    let clipped = extract(&pdf(&format!("q 0 0 600 800 re W n {text} Q"), &[], 0));
    assert_eq!(plain, clipped);
}

#[test]
fn invisible_ocr_text_without_path_clips_is_preserved() {
    assert_text(
        &pdf("BT /F1 10 Tf 3 Tr 10 20 Td (OCR) Tj ET", &[], 0),
        "OCR",
    );
}

#[test]
fn disjoint_clips_hide_all_glyphs() {
    assert_text(
        &pdf(
            "q 0 0 5 5 re W n 10 10 5 5 re W n BT /F1 10 Tf 10 20 Td (HIDDEN) Tj ET Q",
            &[],
            0,
        ),
        "",
    );
}

#[test]
fn content_matrix_is_not_applied_to_clip_twice() {
    assert_text(
        &pdf(
            "q 2 0 0 2 100 100 cm 0 0 14 40 re W n BT /F1 10 Tf 0 20 Td (ABCDE) Tj ET Q",
            &[],
            0,
        ),
        "ABC",
    );
}

#[test]
fn unsupported_parent_clip_preserves_form_text() {
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /Font << /F1 4 0 R >> >>",
        "BT /F1 10 Tf 10 20 Td (KEEP) Tj ET",
    );
    assert_text(
        &pdf("0 0 m 1 0 l 1 1 l h W n /Outer Do", &[&form], 0),
        "KEEP",
    );
}

#[test]
fn partial_glyphs_keep_the_full_text_and_geometry_at_all_clip_edges() {
    let text = "BT /F1 10 Tf 10 20 Td (A) Tj ET";
    let original = extract(&pdf(text, &[], 0));
    let glyph = original.0.iter().find(|item| item.text == "A").unwrap();
    // Raw item coordinates have a top-left origin; content streams use y-up.
    let left = glyph.x;
    let right = left + glyph.width;
    let top = 800.0 - glyph.y;
    let bottom = top - glyph.height;
    let clips = [
        (left - 1.0, bottom - 1.0, 1.1, glyph.height + 2.0),
        (right - 0.1, bottom - 1.0, 1.1, glyph.height + 2.0),
        (left - 1.0, bottom - 1.0, glyph.width + 2.0, 1.1),
        (left - 1.0, top - 0.1, glyph.width + 2.0, 1.1),
    ];
    for (x, y, width, height) in clips {
        let content = format!("q {x} {y} {width} {height} re W n {text} Q");
        assert_eq!(extract(&pdf(&content, &[], 0)), original);
    }
}
