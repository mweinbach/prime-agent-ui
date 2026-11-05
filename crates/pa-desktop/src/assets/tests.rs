use super::{EMBEDDED_FONTS, REGISTERED, register_embedded_fonts};
use crate::theme::{font_display, font_mono, font_sans, font_serif};
use gpui::TestAppContext;
use std::collections::BTreeSet;

#[test]
fn registers_once_per_app() {
    let app = TestAppContext::single();
    let second_app = TestAppContext::single();
    app.read(register_embedded_fonts);
    app.read(register_embedded_fonts);
    second_app.read(register_embedded_fonts);
    // GPUI's in-memory test backend accepts fonts but has no font database.
    REGISTERED.with_borrow(|entries| assert_eq!(entries.len(), 2));
    second_app.quit();
    app.quit();
}

// Read the handful of big-endian sfnt fields used for platform font matching.
fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn u32_at(bytes: &[u8], offset: usize) -> usize {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize
}

fn table(ttf: &[u8], tag: [u8; 4]) -> Option<&[u8]> {
    let count = usize::from(u16_at(ttf, 4));
    ttf[12..]
        .as_chunks::<16>()
        .0
        .iter()
        .take(count)
        .find_map(|record| {
            if record[..4] != tag {
                return None;
            }
            let start = u32_at(record, 8);
            Some(&ttf[start..start + u32_at(record, 12)])
        })
}

fn names(table: &[u8], id: u16) -> BTreeSet<String> {
    let strings = &table[usize::from(u16_at(table, 4))..];
    let count = usize::from(u16_at(table, 2));
    table[6..]
        .as_chunks::<12>()
        .0
        .iter()
        .take(count)
        .filter(|&record| u16_at(record, 6) == id)
        .map(|record| {
            let start = usize::from(u16_at(record, 10));
            let name = &strings[start..start + usize::from(u16_at(record, 8))];
            match u16_at(record, 0) {
                0 | 3 => {
                    let utf16 = name
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .copied()
                        .map(u16::from_be_bytes)
                        .collect::<Vec<_>>();
                    String::from_utf16(&utf16).unwrap()
                }
                1 => std::str::from_utf8(name).unwrap().to_owned(), // Our names are ASCII.
                platform => panic!("unexpected font name platform {platform}"),
            }
        })
        .collect()
}

#[test]
fn static_faces_match_the_themes_and_native_style_selectors() {
    let families = [
        (
            "Google Sans",
            font_sans(),
            &[
                "Regular",
                "Medium",
                "SemiBold",
                "Bold",
                "Italic",
                "Bold Italic",
            ][..],
        ),
        (
            "Google Sans Code",
            font_mono(),
            &["Regular", "Medium", "SemiBold", "Bold", "Italic"][..],
        ),
        (
            "Google Sans Flex",
            font_display(),
            &["Regular", "Medium", "SemiBold", "Bold"][..],
        ),
    ];
    assert_eq!(font_serif(), font_display());
    let mut expected = BTreeSet::new();
    for (family, theme, styles) in families {
        assert_eq!(theme.as_ref(), family);
        for style in styles {
            expected.insert((family.to_owned(), (*style).to_owned()));
        }
    }
    for &ttf in EMBEDDED_FONTS {
        for &tag in [b"fvar", b"gvar", b"avar", b"HVAR", b"MVAR"] {
            assert!(table(ttf, tag).is_none());
        }
        let name = table(ttf, *b"name").unwrap();
        let family = names(name, 1);
        let style = names(name, 2);
        assert_eq!(family, names(name, 16));
        assert_eq!(style, names(name, 17));
        assert_eq!(family.len(), 1);
        assert_eq!(style.len(), 1);
        let family = family.into_iter().next().unwrap();
        let style = style.into_iter().next().unwrap();
        let full = format!("{family} {style}");
        let postscript = format!("{family}-{style}").replace(' ', "");
        assert_eq!(names(name, 4), BTreeSet::from([full]));
        assert_eq!(names(name, 6), BTreeSet::from([postscript]));

        let weight = match style.as_str() {
            "Bold" | "Bold Italic" => 700,
            "SemiBold" => 600,
            "Medium" => 500,
            _ => 400,
        };
        let italic = u16::from(style.ends_with("Italic"));
        let bold = u16::from(weight == 700);
        let regular = u16::from(style == "Regular");
        let os2 = table(ttf, *b"OS/2").unwrap();
        assert_eq!(u16_at(os2, 4), weight);
        assert_eq!(
            u16_at(os2, 62) & 0x61,
            (regular << 6) | (bold << 5) | italic
        );
        let head = table(ttf, *b"head").unwrap();
        assert_eq!(u16_at(head, 44) & 3, (italic << 1) | bold);
        assert!(expected.remove(&(family, style)));
    }
    assert!(expected.is_empty(), "missing embedded faces: {expected:?}");
}
