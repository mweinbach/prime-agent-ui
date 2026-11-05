use gpui::{Context, Modifiers, Render, TestAppContext, Window};

use super::*;
use crate::theme::ThemeMode;

fn children(block: &Block) -> &[Block] {
    let Block::Group(_, children) = block else {
        panic!("expected group: {block:?}")
    };
    children
}

fn text(block: &Block) -> &Inline {
    let Block::Text(text) = block else {
        panic!("expected inline leaf: {block:?}")
    };
    text
}

#[test]
fn block_containers_keep_nested_list_quote_table_and_code_in_source_order() {
    let doc = document::parse(
        "3. first\n\n   next\n\n   - nested\n\n   > > deep\n\n   ```rs\n   let n = 1;\n   ```\n\n   | a | b |\n   | :- | -: |\n   | `x` | y<br>z |\n\n4. last",
    );
    let list = &children(&doc)[0];
    assert!(matches!(list, Block::Group(Kind::List(Some(3)), _)));
    let items = children(list);
    assert_eq!(items.len(), 2);
    let body = children(&items[0]);
    assert!(matches!(
        body,
        [
            Block::Group(Kind::Paragraph, _),
            Block::Group(Kind::Paragraph, _),
            Block::Group(Kind::List(None), _),
            Block::Group(Kind::Quote(None), _),
            Block::Code(_, _),
            Block::Group(Kind::Table(_), _)
        ]
    ));
    assert_eq!(text(&children(&body[1])[0]).text, "next");
    assert!(matches!(
        children(&body[3]),
        [Block::Group(Kind::Quote(None), _)]
    ));
    assert!(matches!(&body[4], Block::Code(lang, code) if lang == "rs" && code == "let n = 1;"));
    assert!(
        matches!(&body[5], Block::Group(Kind::Table(align), _) if align == &[Alignment::Left, Alignment::Right])
    );
    let row = children(&children(&body[5])[1]);
    let value = text(&children(&row[0])[0]);
    assert_eq!(value.text, "x");
    assert!(value.spans[0].1.code);
    assert_eq!(text(&children(&row[1])[0]).text, "y z");
}

#[test]
fn heading_runs_preserve_nested_links_code_emphasis_and_literal_html() {
    let doc = document::parse(
        "### [**_鳥`猫`_**](https://example.org) ~~old~~ Vec<String> <user_id> Result<T, E><br/>x",
    );
    let heading = &children(&doc)[0];
    assert!(matches!(
        heading,
        Block::Group(Kind::Heading(HeadingLevel::H3), _)
    ));
    let rich = text(&children(heading)[0]);
    assert_eq!(rich.text, "鳥猫 old Vec<String> <user_id> Result<T, E>\nx");
    let p = Palette::for_mode(ThemeMode::Dark);
    let style = TypeStyle {
        family: font_display(),
        weight: FontWeight::SEMIBOLD,
        ink: p.text_primary,
    };
    let (runs, links) = inline_runs(rich, &style, p);
    assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), rich.text.len());
    assert_eq!(links, [(0.."鳥猫".len(), "https://example.org".into())]);
    assert_eq!(runs[0].font.family, font_display());
    assert_eq!(runs[1].font.family, font_mono());
    assert_eq!(runs[1].font.weight, FontWeight::BOLD);
    assert_eq!(runs[1].font.style, FontStyle::Italic);
    assert!(runs[1].underline.is_some());
    assert!(runs.iter().any(|r| r.strikethrough.is_some()));
    let html = document::parse("<section>\n<T>\n</section>");
    assert_eq!(
        text(&children(&children(&html)[0])[0]).text,
        "<section>\n<T>\n</section>"
    );
}

#[test]
fn gfm_alerts_contain_their_task_lists() {
    for (marker, kind) in [
        ("NOTE", BlockQuoteKind::Note),
        ("TIP", BlockQuoteKind::Tip),
        ("IMPORTANT", BlockQuoteKind::Important),
        ("WARNING", BlockQuoteKind::Warning),
        ("CAUTION", BlockQuoteKind::Caution),
    ] {
        let doc = document::parse(&format!("> [!{marker}]\n> - [x] ok\n> - [ ] next"));
        let quote = &children(&doc)[0];
        assert!(matches!(quote, Block::Group(Kind::Quote(Some(k)), _) if *k == kind));
        let tasks = children(&children(quote)[0]);
        for (task, expected) in tasks.iter().zip(["☑ ok", "☐ next"]) {
            let inline = text(&children(task)[0]);
            assert_eq!(inline.text, expected);
            assert!(inline.spans[0].1.checkbox);
        }
        assert_eq!(tasks.len(), 2);
    }
}

#[test]
fn streaming_repairs_only_unfinished_markdown_at_the_tail() {
    let reference = "[ref]: https://example.org\n\nSee [the **API**][ref]";
    for (source, expected) in [
        (reference, reference),
        ("See <span title='['>x", "See <span title='['>x"),
        ("intro\n\n```rus", "intro\n\n"),
        ("````rs\nfn f()\n``", "````rs\nfn f()\n"),
        ("```rs\nlet v = [", "```rs\nlet v = ["),
        ("see `Vec<T>", "see `Vec<T>`"),
        ("see `", "see "),
        ("look [here", "look [here]()"),
        ("look [here](", "look [here]()"),
        ("look [here](htt", "look [here]()"),
        ("look [here](a(b)c)", "look [here](a(b)c)"),
        (r"escaped \`ok", r"escaped \`ok"),
        (
            "before `open\n\nordinary end",
            "before `open\n\nordinary end",
        ),
        ("> ```text\n> [literal", "> ```text\n> [literal"),
    ] {
        assert_eq!(streaming::normalize(source), expected, "{source:?}");
    }
}

struct MarkdownView;

impl Render for MarkdownView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        render_markdown(
            "md",
            "[link](https://example.test)\n\n```rust\nlet x = 1;\n```",
            Palette::for_mode(ThemeMode::Light),
        )
    }
}

#[test]
fn rendered_markdown_opens_links_and_copies_fenced_source() {
    let mut app = TestAppContext::single();
    let (_, cx) = app.add_window_view(|_, _| MarkdownView);
    cx.update(|window, cx| window.draw(cx).clear());
    let link = cx.debug_bounds("md-md-1").unwrap().origin + gpui::point(px(2.), px(2.));
    cx.simulate_click(link, Modifiers::none());
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.test"));
    let copy = cx.debug_bounds("md-md-2-copy").unwrap().center();
    cx.simulate_click(copy, Modifiers::none());
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("let x = 1;")
    );
    app.quit();
}
