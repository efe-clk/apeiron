//! The documents plugin of Apeiron (SEARCH.md §3): the text inside PDF and
//! office files, for search. It sees only the bytes it is given and runs in
//! the sandbox of the plugin host (PLUGIN.md §3), which is the right place for
//! parsing files that may come from anyone.

use std::io::{Cursor, Read};

use quick_xml::Reader;
use quick_xml::events::Event;

wit_bindgen::generate!({ path: "../../crates/apeiron-plugin/wit", world: "type-driver-plugin" });

use exports::apeiron::plugin::type_driver::{Guest, Info};

/// The names this plugin finds text in. The content decides how it is read.
const PATTERNS: &[&str] = &[
    "*.pdf", "*.docx", "*.xlsx", "*.pptx", "*.odt", "*.ods", "*.odp",
];

/// The most text one file gives, so that a hostile file cannot fill the
/// memory of the search that asked.
const MAX_TEXT: usize = 32 << 20;

/// The text of a PDF or office file; `None` when it holds none, as a scanned
/// PDF does, or is neither kind of file.
pub fn extract(content: &[u8]) -> Result<Option<String>, String> {
    let text = if content.starts_with(b"%PDF") {
        pdf(content)?
    } else if content.starts_with(b"PK") {
        office(content)?
    } else {
        String::new()
    };
    Ok((!text.trim().is_empty()).then_some(text))
}

fn pdf(content: &[u8]) -> Result<String, String> {
    let document = lopdf::Document::load_mem(content).map_err(|e| e.to_string())?;
    let mut text = String::new();
    for number in document.get_pages().into_keys() {
        // A page that cannot be read does not lose the others.
        if let Ok(page) = document.extract_text(&[number]) {
            text.push_str(&page);
            text.push('\n');
        }
        if text.len() >= MAX_TEXT {
            break;
        }
    }
    Ok(text)
}

/// Where a kind of office file keeps its text.
struct Part {
    /// Whether an entry of the zip file holds text.
    holds: fn(&str) -> bool,
    /// Whether the text inside an element counts.
    keeps: fn(&[u8]) -> bool,
    /// Whether the end of an element ends a line.
    breaks: fn(&[u8]) -> bool,
}

fn is_t(name: &[u8]) -> bool {
    name == b"t"
}

const WORD: Part = Part {
    holds: |n| n == "word/document.xml",
    keeps: is_t,
    breaks: |e| e == b"p",
};
const EXCEL: Part = Part {
    // Cells that are numbers are left out: what people search for are words.
    holds: |n| n == "xl/sharedStrings.xml" || n.starts_with("xl/worksheets/"),
    keeps: is_t,
    breaks: |e| e == b"si" || e == b"c",
};
const POWERPOINT: Part = Part {
    holds: |n| n.starts_with("ppt/slides/slide"),
    keeps: is_t,
    breaks: |e| e == b"p",
};
const OPEN_DOCUMENT: Part = Part {
    holds: |n| n == "content.xml",
    keeps: |e| e == b"p" || e == b"h",
    breaks: |e| e == b"p" || e == b"h",
};

fn office(content: &[u8]) -> Result<String, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(content)).map_err(|e| e.to_string())?;
    let mut names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    names.sort();
    let has = |n: &str| names.iter().any(|x| x == n);
    let part = if has("word/document.xml") {
        WORD
    } else if has("xl/workbook.xml") {
        EXCEL
    } else if has("ppt/presentation.xml") {
        POWERPOINT
    } else if has("content.xml") {
        OPEN_DOCUMENT
    } else {
        return Ok(String::new());
    };
    let mut text = String::new();
    for name in names.iter().filter(|n| (part.holds)(n)) {
        let mut xml = Vec::new();
        archive
            .by_name(name)
            .map_err(|e| e.to_string())?
            .take(MAX_TEXT as u64 * 4)
            .read_to_end(&mut xml)
            .map_err(|e| e.to_string())?;
        xml_text(&xml, &part, &mut text)?;
        if text.len() >= MAX_TEXT {
            break;
        }
    }
    Ok(text)
}

fn xml_text(xml: &[u8], part: &Part, out: &mut String) -> Result<(), String> {
    let mut reader = Reader::from_reader(xml);
    let mut inside = 0usize;
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) => {
                if (part.keeps)(e.local_name().as_ref()) {
                    inside += 1;
                }
            }
            Event::End(e) => {
                let name = e.local_name();
                if (part.keeps)(name.as_ref()) {
                    inside = inside.saturating_sub(1);
                }
                if (part.breaks)(name.as_ref()) && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::Text(t) if inside > 0 => {
                out.push_str(&t.decode().map_err(|e| e.to_string())?);
            }
            Event::CData(t) if inside > 0 => {
                out.push_str(&String::from_utf8_lossy(&t));
            }
            Event::GeneralRef(r) if inside > 0 => {
                let name = r.decode().map_err(|e| e.to_string())?;
                if let Some(c) = reference(&name) {
                    out.push(c);
                }
            }
            Event::Eof => return Ok(()),
            _ => {}
        }
        if out.len() >= MAX_TEXT {
            return Ok(());
        }
    }
}

/// The character of `&name;`: the five XML defines and numbered ones. Others
/// are left out, so that a file cannot grow by defining its own.
fn reference(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix('x') {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

struct Documents;

impl Guest for Documents {
    fn describe() -> Info {
        Info {
            name: "documents".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Finds the text inside PDF and office files".into(),
            patterns: Vec::new(),
            text_patterns: PATTERNS.iter().map(|p| (*p).to_owned()).collect(),
            permissions: Vec::new(),
        }
    }

    fn merge(_: Option<Vec<u8>>, _: Vec<u8>, _: Vec<u8>) -> Result<Option<Vec<u8>>, String> {
        Ok(None)
    }

    fn diff(_: Vec<u8>, _: Vec<u8>) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn extract_text(content: Vec<u8>) -> Result<Option<String>, String> {
        extract(&content)
    }
}

#[cfg(target_arch = "wasm32")]
export!(Documents);

#[cfg(test)]
mod tests {
    use std::io::Write;

    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    use super::*;

    fn zip_of(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        let options = zip::write::SimpleFileOptions::default();
        for (name, text) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        out.into_inner()
    }

    fn pdf_of(line: &str) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
        });
        let resources = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font },
        });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![50.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal(line)]),
                Operation::new("ET", vec![]),
            ],
        };
        let contents = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => contents,
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page.into()],
                "Count" => 1,
                "Resources" => resources,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn pdf_text_is_found() {
        let text = extract(&pdf_of("Invoice for October")).unwrap().unwrap();
        assert!(text.contains("Invoice for October"), "{text:?}");
    }

    #[test]
    fn word_keeps_paragraphs_and_special_characters() {
        let xml = r#"<w:document xmlns:w="w"><w:body>
            <w:p><w:r><w:t>Fatura &amp; </w:t><w:t>makbuz</w:t></w:r></w:p>
            <w:p><w:r><w:instrText>HIDDEN</w:instrText><w:t>Ayşe &#231;</w:t></w:r></w:p>
            </w:body></w:document>"#;
        let text = extract(&zip_of(&[("word/document.xml", xml)]))
            .unwrap()
            .unwrap();
        assert_eq!(text.trim(), "Fatura & makbuz\nAyşe ç");
    }

    #[test]
    fn excel_powerpoint_and_open_document() {
        let excel = zip_of(&[
            ("xl/workbook.xml", "<workbook/>"),
            (
                "xl/sharedStrings.xml",
                "<sst><si><t>Kira</t></si><si><t>Gider</t></si></sst>",
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<s><c t="s"><v>0</v></c><c t="inlineStr"><is><t>Satır</t></is></c></s>"#,
            ),
        ]);
        let text = extract(&excel).unwrap().unwrap();
        assert!(text.contains("Kira") && text.contains("Gider") && text.contains("Satır"));
        assert!(!text.contains('0'), "{text:?}");

        let slides = zip_of(&[
            ("ppt/presentation.xml", "<p/>"),
            (
                "ppt/slides/slide1.xml",
                "<sld><a:p><a:r><a:t>Birinci</a:t></a:r></a:p></sld>",
            ),
            ("ppt/slides/_rels/slide1.xml.rels", "<a:t>not this</a:t>"),
        ]);
        assert_eq!(extract(&slides).unwrap().unwrap().trim(), "Birinci");

        let odt = zip_of(&[(
            "content.xml",
            "<office:body><text:h>Başlık</text:h><text:p>Metin <text:span>burada</text:span></text:p></office:body>",
        )]);
        assert_eq!(
            extract(&odt).unwrap().unwrap().trim(),
            "Başlık\nMetin burada"
        );
    }

    #[test]
    fn what_is_not_a_document_gives_nothing_or_an_error() {
        assert_eq!(extract(b"plain text").unwrap(), None);
        assert_eq!(extract(&zip_of(&[("a.txt", "x")])).unwrap(), None);
        assert!(extract(b"PK\x03\x04not really a zip file").is_err());
        assert!(extract(b"%PDF-1.5 not really a pdf").is_err());
        let empty = zip_of(&[("word/document.xml", "<w:document/>")]);
        assert_eq!(extract(&empty).unwrap(), None);
    }

    #[test]
    fn a_hostile_entity_definition_adds_nothing() {
        let xml = r#"<!DOCTYPE d [<!ENTITY x "xxxxxxxxxx">]>
            <w:document><w:p><w:t>a&x;&x;b</w:t></w:p></w:document>"#;
        let text = extract(&zip_of(&[("word/document.xml", xml)]))
            .unwrap()
            .unwrap();
        assert_eq!(text.trim(), "ab");
    }
}
