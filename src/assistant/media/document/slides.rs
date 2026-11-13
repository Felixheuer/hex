//! A PowerPoint deck (16:9) written as Office Open XML: each slide has its
//! title, its text (paragraphs, bullets, numbered items, sub-headings),
//! and its tables and pictures beside or below the text; speaker notes go
//! into notes pages. Every shape is placed explicitly, so the deck looks
//! the same in PowerPoint, Keynote, and Quick Look.

use std::fmt::Write as _;
use std::path::Path;

use super::blocks::{Block, Slide, runs};
use super::write::{EMU_PER_PX, content_types, core, picture, rels, root_rels, xml};
use crate::assistant::media::out;
use crate::assistant::media::zip::Writer;

/// The slide, in EMU (13.333 × 7.5 inches), and its margin.
const WIDTH: u64 = 12_192_000;
const HEIGHT: u64 = 6_858_000;
const MARGIN: u64 = 457_200;
const TITLE_HEIGHT: u64 = 1_005_840;

const NAMESPACES: &str = "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"";
const XML_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const GROUP: &str = "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";
const COLOR_MAP: &str = "<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>";

fn theme() -> String {
    let color =
        |name: &str, value: &str| format!("<a:{name}><a:srgbClr val=\"{value}\"/></a:{name}>");
    let fill = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>";
    let line = "<a:ln w=\"9525\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>";
    format!(
        "{XML_HEAD}<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Agent-Desk\"><a:themeElements><a:clrScheme name=\"Agent-Desk\"><a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1><a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1>{}{}{}{}{}{}{}{}{}{}</a:clrScheme><a:fontScheme name=\"Agent-Desk\"><a:majorFont><a:latin typeface=\"Helvetica Neue\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont><a:minorFont><a:latin typeface=\"Helvetica Neue\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme><a:fmtScheme name=\"Agent-Desk\"><a:fillStyleLst>{fill}{fill}{fill}</a:fillStyleLst><a:lnStyleLst>{line}{line}{line}</a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst>{fill}{fill}{fill}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>",
        color("dk2", "1F2937"),
        color("lt2", "F3F4F6"),
        color("accent1", "2563EB"),
        color("accent2", "DC2626"),
        color("accent3", "16A34A"),
        color("accent4", "D97706"),
        color("accent5", "7C3AED"),
        color("accent6", "0891B2"),
        color("hlink", "2563EB"),
        color("folHlink", "7C3AED"),
    )
}

fn runs_xml(text: &str, size: u32, bold: bool) -> String {
    let mut out = String::new();
    for (part, strong) in runs(text) {
        let _ = write!(
            out,
            "<a:r><a:rPr lang=\"en-US\" sz=\"{size}\"{} dirty=\"0\"/><a:t>{}</a:t></a:r>",
            if bold || strong { " b=\"1\"" } else { "" },
            xml(&part)
        );
    }
    out
}

/// A text box at `(x, y)` of `(cx, cy)`; `title` makes it the slide's
/// title placeholder.
fn text_shape(
    id: usize,
    (x, y, cx, cy): (u64, u64, u64, u64),
    paragraphs: &str,
    title: bool,
    anchor: &str,
) -> String {
    let (name, props, nv) = if title {
        (
            "Title",
            "<p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr>",
            "<p:nvPr><p:ph type=\"title\"/></p:nvPr>",
        )
    } else {
        ("Text", "<p:cNvSpPr txBox=\"1\"/>", "<p:nvPr/>")
    };
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name} {id}\"/>{props}{nv}</p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap=\"square\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" anchor=\"{anchor}\"><a:normAutofit/></a:bodyPr><a:lstStyle/>{paragraphs}</p:txBody></p:sp>"
    )
}

/// The slide's text blocks as paragraphs, and how many lines they take.
fn paragraphs(blocks: &[&Block], size: u32) -> String {
    let mut out = String::new();
    let plain = "<a:pPr marL=\"0\" indent=\"0\"><a:spcBef><a:spcPts val=\"600\"/></a:spcBef><a:buNone/></a:pPr>";
    for block in blocks {
        match block {
            Block::Heading { text, .. } => {
                let _ = write!(
                    out,
                    "<a:p>{plain}{}</a:p>",
                    runs_xml(text, size + 200, true)
                );
            }
            Block::Paragraph(text) | Block::Quote(text) => {
                let _ = write!(out, "<a:p>{plain}{}</a:p>", runs_xml(text, size, false));
            }
            Block::Code(text) => {
                for line in text.lines() {
                    let _ = write!(
                        out,
                        "<a:p><a:pPr><a:buNone/></a:pPr><a:r><a:rPr lang=\"en-US\" sz=\"{}\" dirty=\"0\"><a:latin typeface=\"Menlo\"/></a:rPr><a:t>{}</a:t></a:r></a:p>",
                        size.saturating_sub(400).max(1000),
                        xml(line)
                    );
                }
            }
            Block::Bullets { items, ordered } => {
                let mark = if *ordered {
                    "<a:buFont typeface=\"+mj-lt\"/><a:buAutoNum type=\"arabicPeriod\"/>"
                } else {
                    "<a:buFont typeface=\"Arial\"/><a:buChar char=\"•\"/>"
                };
                for item in items {
                    let _ = write!(
                        out,
                        "<a:p><a:pPr marL=\"342900\" indent=\"-342900\"><a:spcBef><a:spcPts val=\"600\"/></a:spcBef>{mark}</a:pPr>{}</a:p>",
                        runs_xml(item, size, false)
                    );
                }
            }
            Block::Table { .. } | Block::Image { .. } | Block::PageBreak => {}
        }
    }
    if out.is_empty() {
        out.push_str("<a:p><a:endParaRPr lang=\"en-US\"/></a:p>");
    }
    out
}

/// Lines the text blocks take, roughly, for the font size.
fn lines(blocks: &[&Block]) -> usize {
    blocks
        .iter()
        .map(|block| match block {
            Block::Heading { text, .. } | Block::Paragraph(text) | Block::Quote(text) => {
                1 + text.chars().count() / 70
            }
            Block::Code(text) => text.lines().count(),
            Block::Bullets { items, .. } => {
                items.iter().map(|item| 1 + item.chars().count() / 70).sum()
            }
            _ => 0,
        })
        .sum()
}

fn table_shape(
    id: usize,
    rows: &[Vec<String>],
    header: bool,
    (x, y, cx, cy): (u64, u64, u64, u64),
) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let column_width = cx / columns as u64;
    let row_height = (cy / rows.len().max(1) as u64).min(457_200);
    let size = if rows.len() > 10 || columns > 5 {
        1100
    } else {
        1400
    };
    let border = |side: &str| {
        format!(
            "<a:{side} w=\"6350\"><a:solidFill><a:srgbClr val=\"A3A3A3\"/></a:solidFill></a:{side}>"
        )
    };
    let borders = format!(
        "{}{}{}{}",
        border("lnL"),
        border("lnR"),
        border("lnT"),
        border("lnB")
    );
    let mut table = format!(
        "<a:tbl><a:tblPr firstRow=\"{}\" bandRow=\"0\"/><a:tblGrid>{}</a:tblGrid>",
        u8::from(header),
        format!("<a:gridCol w=\"{column_width}\"/>").repeat(columns)
    );
    for (index, row) in rows.iter().enumerate() {
        let head = header && index == 0;
        let _ = write!(table, "<a:tr h=\"{row_height}\">");
        for column in 0..columns {
            let text = row.get(column).map_or("", String::as_str);
            let fill = if head {
                "<a:solidFill><a:srgbClr val=\"E5E7EB\"/></a:solidFill>"
            } else {
                ""
            };
            let _ = write!(
                table,
                "<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p>{}</a:p></a:txBody><a:tcPr>{borders}{fill}</a:tcPr></a:tc>",
                if text.is_empty() {
                    format!("<a:endParaRPr lang=\"en-US\" sz=\"{size}\"/>")
                } else {
                    runs_xml(text, size, head)
                }
            );
        }
        table.push_str("</a:tr>");
    }
    table.push_str("</a:tbl>");
    format!(
        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Table {id}\"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp=\"1\"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{}\" cy=\"{}\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\">{table}</a:graphicData></a:graphic></p:graphicFrame>",
        column_width * columns as u64,
        row_height * rows.len() as u64
    )
}

fn picture_shape(
    id: usize,
    relation: &str,
    size: (u32, u32),
    (x, y, cx, cy): (u64, u64, u64, u64),
) -> String {
    // Fit inside the box, centered, keeping the aspect ratio.
    let (width, height) = (u64::from(size.0.max(1)), u64::from(size.1.max(1)));
    let natural = (width * EMU_PER_PX, height * EMU_PER_PX);
    let scale = (cx as f64 / natural.0 as f64)
        .min(cy as f64 / natural.1 as f64)
        .min(2.0);
    let (w, h) = (
        (natural.0 as f64 * scale) as u64,
        (natural.1 as f64 * scale) as u64,
    );
    let (left, top) = (x + (cx - w.min(cx)) / 2, y + (cy - h.min(cy)) / 2);
    format!(
        "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"Picture {id}\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"{relation}\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x=\"{left}\" y=\"{top}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>"
    )
}

/// One slide's XML and its pictures (relation id, file name, bytes).
fn slide_xml(
    slide: &Slide,
    base: &Path,
    media: &mut Vec<(String, Vec<u8>)>,
) -> Result<(String, Vec<(String, String)>), String> {
    let mut shapes = String::new();
    let mut pictures = Vec::new();
    let mut id = 2;
    let text: Vec<&Block> = slide
        .blocks
        .iter()
        .filter(|block| {
            !matches!(
                block,
                Block::Table { .. } | Block::Image { .. } | Block::PageBreak
            )
        })
        .collect();
    let visuals: Vec<&Block> = slide
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::Table { .. } | Block::Image { .. }))
        .collect();
    let body_top = MARGIN + TITLE_HEIGHT + 91_440;
    let body_height = HEIGHT - body_top - MARGIN;
    let inner = WIDTH - 2 * MARGIN;
    if text.is_empty() && visuals.is_empty() {
        // A title slide.
        shapes.push_str(&text_shape(
            id,
            (MARGIN, HEIGHT / 2 - TITLE_HEIGHT, inner, TITLE_HEIGHT * 2),
            &format!(
                "<a:p><a:pPr algn=\"ctr\"/>{}</a:p>",
                runs_xml(&slide.title, 4400, true)
            ),
            true,
            "ctr",
        ));
    } else {
        if !slide.title.is_empty() {
            shapes.push_str(&text_shape(
                id,
                (MARGIN, MARGIN, inner, TITLE_HEIGHT),
                &format!("<a:p>{}</a:p>", runs_xml(&slide.title, 3200, true)),
                true,
                "b",
            ));
            id += 1;
        }
        let (text_width, visual_left) = match (text.is_empty(), visuals.is_empty()) {
            (false, false) => (inner * 45 / 100, MARGIN + inner * 47 / 100),
            (false, true) => (inner, 0),
            _ => (0, MARGIN),
        };
        if !text.is_empty() {
            let size = match lines(&text) {
                0..=6 => 2400,
                7..=10 => 2000,
                11..=15 => 1600,
                _ => 1400,
            };
            shapes.push_str(&text_shape(
                id,
                (MARGIN, body_top, text_width, body_height),
                &paragraphs(&text, size),
                false,
                "t",
            ));
            id += 1;
        }
        let visual_width = MARGIN + inner - visual_left;
        let each = body_height / visuals.len().max(1) as u64;
        for (index, visual) in visuals.iter().enumerate() {
            let area = (
                visual_left,
                body_top + each * index as u64,
                visual_width,
                each.saturating_sub(91_440),
            );
            match visual {
                Block::Table { rows, header } => {
                    shapes.push_str(&table_shape(id, rows, *header, area))
                }
                Block::Image { path, .. } => {
                    let (bytes, extension, size) = picture(&out::resolve(path, base))?;
                    let name = format!("image{}.{extension}", media.len() + 1);
                    let relation = format!("rIdImage{}", pictures.len() + 1);
                    shapes.push_str(&picture_shape(id, &relation, size, area));
                    pictures.push((relation, name.clone()));
                    media.push((name, bytes));
                }
                _ => {}
            }
            id += 1;
        }
    }
    Ok((
        format!(
            "{XML_HEAD}<p:sld {NAMESPACES}><p:cSld><p:spTree>{GROUP}{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
        ),
        pictures,
    ))
}

fn notes_xml(notes: &str) -> String {
    let paragraphs: String = notes
        .lines()
        .map(|line| format!("<a:p>{}</a:p>", runs_xml(line, 1200, false)))
        .collect();
    format!(
        "{XML_HEAD}<p:notes {NAMESPACES}><p:cSld><p:spTree>{GROUP}<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Notes\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"body\" idx=\"1\"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"685800\" y=\"4800600\"/><a:ext cx=\"5486400\" cy=\"3600450\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/>{paragraphs}</p:txBody></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>"
    )
}

/// The deck: `title` for its properties, `slides` in order.
pub fn pptx(title: &str, slides: &[Slide], base: &Path) -> Result<Vec<u8>, String> {
    let mut zip = Writer::default();
    let mut media: Vec<(String, Vec<u8>)> = Vec::new();
    let with_notes = slides.iter().any(|slide| !slide.notes.trim().is_empty());
    let mut overrides = vec![
        (
            "/ppt/presentation.xml".to_owned(),
            "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
        ),
        (
            "/ppt/slideMasters/slideMaster1.xml".to_owned(),
            "application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml",
        ),
        (
            "/ppt/slideLayouts/slideLayout1.xml".to_owned(),
            "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml",
        ),
        (
            "/ppt/theme/theme1.xml".to_owned(),
            "application/vnd.openxmlformats-officedocument.theme+xml",
        ),
        (
            "/ppt/presProps.xml".to_owned(),
            "application/vnd.openxmlformats-officedocument.presentationml.presProps+xml",
        ),
        (
            "/ppt/viewProps.xml".to_owned(),
            "application/vnd.openxmlformats-officedocument.presentationml.viewProps+xml",
        ),
        (
            "/ppt/tableStyles.xml".to_owned(),
            "application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml",
        ),
    ];
    let mut presentation_rels = vec![(
        "rId1".to_owned(),
        "slideMaster".to_owned(),
        "slideMasters/slideMaster1.xml".to_owned(),
    )];
    let mut slide_ids = String::new();
    for (index, slide) in slides.iter().enumerate() {
        let n = index + 1;
        let (xml_text, pictures) = slide_xml(slide, base, &mut media)?;
        zip.add(&format!("ppt/slides/slide{n}.xml"), xml_text.as_bytes());
        let mut relations = vec![(
            "rId1".to_owned(),
            "slideLayout".to_owned(),
            "../slideLayouts/slideLayout1.xml".to_owned(),
        )];
        for (relation, name) in pictures {
            relations.push((relation, "image".into(), format!("../media/{name}")));
        }
        if !slide.notes.trim().is_empty() {
            relations.push((
                "rIdNotes".into(),
                "notesSlide".into(),
                format!("../notesSlides/notesSlide{n}.xml"),
            ));
            zip.add(
                &format!("ppt/notesSlides/notesSlide{n}.xml"),
                notes_xml(&slide.notes).as_bytes(),
            );
            zip.add(
                &format!("ppt/notesSlides/_rels/notesSlide{n}.xml.rels"),
                rels(&[
                    (
                        "rId1".into(),
                        "notesMaster".into(),
                        "../notesMasters/notesMaster1.xml".into(),
                    ),
                    (
                        "rId2".into(),
                        "slide".into(),
                        format!("../slides/slide{n}.xml"),
                    ),
                ])
                .as_bytes(),
            );
            overrides.push((
                format!("/ppt/notesSlides/notesSlide{n}.xml"),
                "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml",
            ));
        }
        zip.add(
            &format!("ppt/slides/_rels/slide{n}.xml.rels"),
            rels(&relations).as_bytes(),
        );
        overrides.push((
            format!("/ppt/slides/slide{n}.xml"),
            "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
        ));
        presentation_rels.push((
            format!("rId{}", n + 1),
            "slide".into(),
            format!("slides/slide{n}.xml"),
        ));
        let _ = write!(
            slide_ids,
            "<p:sldId id=\"{}\" r:id=\"rId{}\"/>",
            255 + n,
            n + 1
        );
    }
    let after = slides.len() + 2;
    presentation_rels.push((
        format!("rId{after}"),
        "theme".into(),
        "theme/theme1.xml".into(),
    ));
    presentation_rels.push((
        format!("rId{}", after + 1),
        "presProps".into(),
        "presProps.xml".into(),
    ));
    presentation_rels.push((
        format!("rId{}", after + 2),
        "viewProps".into(),
        "viewProps.xml".into(),
    ));
    presentation_rels.push((
        format!("rId{}", after + 3),
        "tableStyles".into(),
        "tableStyles.xml".into(),
    ));
    let notes_master = if with_notes {
        let id = format!("rId{}", after + 4);
        presentation_rels.push((
            id.clone(),
            "notesMaster".into(),
            "notesMasters/notesMaster1.xml".into(),
        ));
        overrides.push((
            "/ppt/notesMasters/notesMaster1.xml".into(),
            "application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml",
        ));
        overrides.push((
            "/ppt/theme/theme2.xml".into(),
            "application/vnd.openxmlformats-officedocument.theme+xml",
        ));
        zip.add(
            "ppt/notesMasters/notesMaster1.xml",
            format!("{XML_HEAD}<p:notesMaster {NAMESPACES}><p:cSld><p:spTree>{GROUP}</p:spTree></p:cSld>{COLOR_MAP}</p:notesMaster>").as_bytes(),
        );
        zip.add(
            "ppt/notesMasters/_rels/notesMaster1.xml.rels",
            rels(&[("rId1".into(), "theme".into(), "../theme/theme2.xml".into())]).as_bytes(),
        );
        zip.add("ppt/theme/theme2.xml", theme().as_bytes());
        format!("<p:notesMasterIdLst><p:notesMasterId r:id=\"{id}\"/></p:notesMasterIdLst>")
    } else {
        String::new()
    };
    let presentation = format!(
        "{XML_HEAD}<p:presentation {NAMESPACES} saveSubsetFonts=\"1\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst>{notes_master}<p:sldIdLst>{slide_ids}</p:sldIdLst><p:sldSz cx=\"{WIDTH}\" cy=\"{HEIGHT}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/><p:defaultTextStyle><a:defPPr><a:defRPr lang=\"en-US\"/></a:defPPr></p:defaultTextStyle></p:presentation>"
    );
    for (name, bytes) in &media {
        zip.add(&format!("ppt/media/{name}"), bytes);
    }
    zip.add("[Content_Types].xml", content_types(&overrides).as_bytes());
    zip.add(
        "_rels/.rels",
        root_rels("ppt/presentation.xml", "officeDocument").as_bytes(),
    );
    zip.add("docProps/core.xml", core(title).as_bytes());
    zip.add("ppt/presentation.xml", presentation.as_bytes());
    zip.add(
        "ppt/_rels/presentation.xml.rels",
        rels(&presentation_rels).as_bytes(),
    );
    zip.add(
        "ppt/slideMasters/slideMaster1.xml",
        format!(
            "{XML_HEAD}<p:sldMaster {NAMESPACES}><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg><p:spTree>{GROUP}</p:spTree></p:cSld>{COLOR_MAP}<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr sz=\"3200\" b=\"1\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mj-lt\"/></a:defRPr></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr><a:defRPr sz=\"2000\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/></a:defRPr></a:lvl1pPr></p:bodyStyle><p:otherStyle><a:lvl1pPr><a:defRPr sz=\"1800\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/></a:defRPr></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>"
        )
        .as_bytes(),
    );
    zip.add(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        rels(&[
            (
                "rId1".into(),
                "slideLayout".into(),
                "../slideLayouts/slideLayout1.xml".into(),
            ),
            ("rId2".into(), "theme".into(), "../theme/theme1.xml".into()),
        ])
        .as_bytes(),
    );
    zip.add(
        "ppt/slideLayouts/slideLayout1.xml",
        format!(
            "{XML_HEAD}<p:sldLayout {NAMESPACES} type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\"><p:spTree>{GROUP}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
        )
        .as_bytes(),
    );
    zip.add(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        rels(&[(
            "rId1".into(),
            "slideMaster".into(),
            "../slideMasters/slideMaster1.xml".into(),
        )])
        .as_bytes(),
    );
    zip.add("ppt/theme/theme1.xml", theme().as_bytes());
    zip.add(
        "ppt/presProps.xml",
        format!("{XML_HEAD}<p:presentationPr {NAMESPACES}/>").as_bytes(),
    );
    zip.add(
        "ppt/viewProps.xml",
        format!("{XML_HEAD}<p:viewPr {NAMESPACES}><p:normalViewPr><p:restoredLeft sz=\"15620\"/><p:restoredTop sz=\"94660\"/></p:normalViewPr><p:gridSpacing cx=\"76200\" cy=\"76200\"/></p:viewPr>").as_bytes(),
    );
    zip.add(
        "ppt/tableStyles.xml",
        format!("{XML_HEAD}<a:tblStyleLst xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" def=\"{{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}}\"/>").as_bytes(),
    );
    Ok(zip.finish())
}
