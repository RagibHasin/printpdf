use std::rc::Rc;

use allsorts::binary::read::ReadScope;
use allsorts::error::{ParseError, ShapingError};
use allsorts::gsub::{self, FeatureMask, Features, GlyphOrigin, RawGlyph};
use allsorts::scripts::preprocess_text;
use allsorts::tables::{cmap::CmapSubtable, FontTableProvider};
use allsorts::{Font, DOTTED_CIRCLE};

// Variant of `bin/shape::map_glyph`
pub fn map_glyph(
    cmap_subtable: &CmapSubtable,
    ch: char,
) -> Result<Option<RawGlyph<()>>, ParseError> {
    // If the ZWNJ glyph is missing, try to fake its existence using the SPACE
    // U+0020 glyph. Mimics Prince's behaviour.
    if ch == '\u{200C}' {
        match cmap_subtable.map_glyph(ch as u32)? {
            Some(index) => Ok(Some(make_glyph(ch, index))),
            None => match cmap_subtable.map_glyph(0x0020)? {
                Some(index) => Ok(Some(make_glyph(ch, index))),
                None => Ok(None),
            },
        }
    } else {
        cmap_subtable
            .map_glyph(ch as u32)
            .map(|opt_index| opt_index.map(|index| make_glyph(ch, index)))
    }
}

// Copy of `bin/shape::make_glyph`
pub fn make_glyph(ch: char, glyph_index: u16) -> RawGlyph<()> {
    RawGlyph {
        unicodes: [ch].into(),
        glyph_index,
        liga_component_pos: 0,
        glyph_origin: GlyphOrigin::Char(ch),
        small_caps: false,
        multi_subst_dup: false,
        is_vert_alt: false,
        fake_bold: false,
        fake_italic: false,
        extra_data: (),
        variation: None,
    }
}

// Variant of `bin/shape::shape_ttf`
pub fn shape_ttf_indic<T: FontTableProvider>(
    font: &mut Font<T>,
    script_tag: u32,
    opt_lang_tag: Option<u32>,
    text: &str,
) -> Result<Vec<u16>, ShapingError> {
    let cmap_subtable_data = font.cmap_subtable_data().to_vec();
    let cmap_subtable = ReadScope::new(&cmap_subtable_data)
        .read::<CmapSubtable<'_>>()
        .expect("no suitable cmap subtable");

    let mut chars = text.chars().collect();
    preprocess_text(&mut chars, script_tag);

    let res_opt_glyphs: Result<Vec<_>, _> = chars
        .iter()
        .map(|ch| map_glyph(&cmap_subtable, *ch))
        .collect();
    let mut opt_glyphs = res_opt_glyphs?;

    // Mimic the existing behaviour of Prince, which is to split a sequence if
    // a font is missing a character glyph. We previously copied the behaviour
    // in `shape.rs`, where the missing glyphs are merely omitted. This can be
    // misleading, especially when comparing the glyph indices as generated by
    // the corpus test against the PDF as generated by Prince.
    //
    // Example:
    //   Assumptions:
    //     1. A and B can form a ligature A+B iff D is base.
    //     2. A+B and C can form a ligature A+B+C iff D is base.
    //
    //   Test sequence: [A, B, Missing, C, D]
    //          Prince: [A, B] [Missing] [C, D] - No ligation. The sequence is split, and
    //                                            D is no longer the base of A and B
    //        shape.rs: [A+B+C, D] - Unexpected ligature; doesn't match Prince's PDF output
    let mut glyphs: Vec<Vec<RawGlyph<()>>> = Vec::new();
    while !opt_glyphs.is_empty() {
        let i = opt_glyphs
            .iter()
            .position(Option::is_none)
            .unwrap_or(opt_glyphs.len() - 1);

        let sub = opt_glyphs.drain(..=i).flatten().collect(); // `flatten` removes any end `None`s
        glyphs.push(sub);
    }

    let gsub_cache = font
        .gsub_cache()
        .expect("unable to get gsub cache")
        .expect("missing gsub table");
    let gdef_table = font.gdef_table().expect("unable to get gdef table");

    let dotted_circle_index = cmap_subtable.map_glyph(DOTTED_CIRCLE as u32)?.unwrap_or(0);
    for gs in glyphs.iter_mut() {
        gsub::apply(
            dotted_circle_index,
            &gsub_cache,
            gdef_table.as_ref().map(Rc::as_ref),
            script_tag,
            opt_lang_tag,
            &Features::Mask(FeatureMask::default()),
            font.num_glyphs(),
            gs,
        )?;
    }

    let glyph_indices = glyphs
        .into_iter()
        .flatten()
        .map(|g| g.glyph_index)
        .collect();

    Ok(glyph_indices)
}
