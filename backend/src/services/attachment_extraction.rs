//! Bounded, killable parsing of untrusted uploads. No parser diagnostics escape.
use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::{
    io::{Cursor, Read, Write},
    process::Stdio,
    sync::LazyLock,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Semaphore,
};

pub const MAX_BYTES: usize = nyxid_machine::MAX_ATTACHMENT_UPLOAD_BYTES;
const MAX_TEXT: usize = 1_000_000;
const MAX_EXPANDED: u64 = 40 * 1024 * 1024;
const MAX_OUTPUT: usize = 8 * 1024 * 1024;
static WORKERS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(2));

#[derive(Clone, Serialize, Deserialize)]
pub struct Section {
    pub offset: usize,
    pub label: String,
}
#[derive(Serialize, Deserialize)]
pub struct Extracted {
    pub content_type: String,
    pub text: String,
    pub sections: Vec<Section>,
}

fn bad(message: &'static str) -> AppError {
    AppError::BadRequest(message.into())
}
fn malformed() -> AppError {
    bad("Malformed or unsupported attachment. Use PDF, DOCX, UTF-8 text, PNG, JPEG, GIF or WebP.")
}

/// Filename is only a hint for text dialects; every hinted structure is validated.
fn parse(bytes: &[u8], extension: &str) -> AppResult<Extracted> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(bad("Upload must contain 1 byte to 20 MiB."));
    }
    if let Ok(format) = image::guess_format(bytes) {
        let mime = match format {
            image::ImageFormat::Png => "image/png",
            image::ImageFormat::Jpeg => "image/jpeg",
            image::ImageFormat::Gif => "image/gif",
            image::ImageFormat::WebP => "image/webp",
            _ => return Err(malformed()),
        };
        let reader = image::ImageReader::with_format(Cursor::new(bytes), format);
        let (w, h) = reader.into_dimensions().map_err(|_| malformed())?;
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 32_000_000 {
            return Err(bad("Image exceeds 32 million pixels."));
        }
        // Full decoding verifies the raster, with a bounded allocation budget.
        let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(192 * 1024 * 1024);
        reader.limits(limits);
        reader.decode().map_err(|_| malformed())?;
        return Ok(Extracted {
            content_type: mime.into(),
            text: String::new(),
            sections: vec![],
        });
    }
    if bytes.starts_with(b"%PDF-") {
        let document = lopdf::Document::load_mem(bytes).map_err(|_| malformed())?;
        if document.is_encrypted() {
            return Err(bad(
                "Encrypted PDFs are not supported. Export an unencrypted copy.",
            ));
        }
        let pages = document.get_pages();
        if pages.is_empty() || pages.len() > 200 {
            return Err(bad("PDF must contain at most 200 pages."));
        }
        let mut out = Extracted {
            content_type: "application/pdf".into(),
            text: String::new(),
            sections: vec![],
        };
        let mut chars = 0;
        for page in pages.keys() {
            let text = document.extract_text(&[*page]).map_err(|_| malformed())?;
            out.sections.push(Section {
                offset: chars,
                label: format!("Page {page}"),
            });
            chars += text.chars().count() + 1;
            if chars > MAX_TEXT {
                return Err(bad("Document exceeds one million extracted characters."));
            }
            out.text.push_str(&text);
            out.text.push('\n');
        }
        return Ok(out);
    }
    if bytes.starts_with(b"PK") {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| malformed())?;
        if archive.len() > 1000 {
            return Err(bad("DOCX contains too many ZIP entries."));
        }
        let mut remaining = MAX_EXPANDED;
        let mut word = None;
        let mut types = None;
        for index in 0..archive.len() {
            let mut file = archive
                .by_index(index)
                .map_err(|_| bad("Encrypted or malformed DOCX. Export an unencrypted copy."))?;
            if file.encrypted() {
                return Err(bad("Encrypted DOCX is not supported."));
            }
            if file.size() > remaining {
                return Err(bad("DOCX exceeds the 40 MiB decompression limit."));
            }
            let name = file.name().to_owned();
            let mut expanded = Vec::new();
            (&mut file)
                .take(remaining + 1)
                .read_to_end(&mut expanded)
                .map_err(|_| malformed())?;
            if expanded.len() as u64 > remaining {
                return Err(bad("DOCX exceeds the 40 MiB decompression limit."));
            }
            remaining -= expanded.len() as u64;
            match name.as_str() {
                "word/document.xml" if word.is_some() => return Err(malformed()),
                "[Content_Types].xml" if types.is_some() => return Err(malformed()),
                "word/document.xml" => word = Some(expanded),
                "[Content_Types].xml" => types = Some(expanded),
                _ => {}
            }
        }

        let types = types.ok_or_else(malformed)?;
        let mut types_reader = quick_xml::Reader::from_reader(types.as_slice());
        let mut declared = false;
        let mut types_root = false;
        let mut structure = XmlStructure::default();
        loop {
            use quick_xml::events::Event;
            let event = types_reader.read_event().map_err(|_| malformed())?;
            structure.check(&event)?;
            match event {
                Event::DocType(_) => return Err(malformed()),
                Event::Start(e) if !types_root => {
                    if e.local_name().as_ref() != b"Types" {
                        return Err(malformed());
                    }
                    types_root = true;
                }
                Event::Empty(e) | Event::Start(e) if e.local_name().as_ref() == b"Override" => {
                    let attributes = e
                        .attributes()
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|_| malformed())?;
                    declared |= attributes.iter().any(|a| a.key.as_ref() == b"PartName" && &a.value[..] == b"/word/document.xml")
                        && attributes.iter().any(|a| a.key.as_ref() == b"ContentType" && &a.value[..] == b"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml");
                }
                Event::Eof => break,
                _ => {}
            }
        }
        if !types_root || !declared {
            return Err(malformed());
        }
        let word = word.ok_or_else(malformed)?;
        let mut reader = quick_xml::Reader::from_reader(word.as_slice());
        let mut text = String::new();
        let mut sections = vec![Section {
            offset: 0,
            label: "Document".into(),
        }];
        let mut document = false;
        let mut in_text = false;
        let mut chars = 0;
        let mut structure = XmlStructure::default();
        loop {
            use quick_xml::events::Event;
            let event = reader.read_event().map_err(|_| malformed())?;
            structure.check(&event)?;
            match event {
                Event::DocType(_) => return Err(malformed()),
                Event::Start(e) if !document => {
                    if e.local_name().as_ref() != b"document" {
                        return Err(malformed());
                    }
                    document = true;
                }
                Event::Start(e) if e.local_name().as_ref() == b"t" => in_text = true,
                Event::End(e) if e.local_name().as_ref() == b"t" => in_text = false,
                Event::Text(e) if in_text => {
                    let decoded = e.xml10_content().map_err(|_| malformed())?;
                    chars += decoded.chars().count();
                    text.push_str(&decoded);
                }
                Event::GeneralRef(e) if in_text => {
                    let name = e.decode().map_err(|_| malformed())?;
                    let escaped = format!("&{name};");
                    let value = quick_xml::escape::unescape(&escaped).map_err(|_| malformed())?;
                    chars += value.chars().count();
                    text.push_str(&value);
                }
                Event::End(e) if e.local_name().as_ref() == b"p" => {
                    text.push('\n');
                    chars += 1;
                    // Coarse paragraph sections keep metadata bounded on large documents.
                    if sections.len() < 200 && chars >= sections.last().unwrap().offset + 5000 {
                        sections.push(Section {
                            offset: chars,
                            label: format!("Section {}", sections.len() + 1),
                        });
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            if chars > MAX_TEXT {
                return Err(bad("Document exceeds one million extracted characters."));
            }
        }
        if !document {
            return Err(malformed());
        }
        return Ok(Extracted {
            content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                .into(),
            text,
            sections,
        });
    }
    // Claimed binary formats cannot become text just because their spoof is UTF-8.
    if matches!(
        extension,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "pdf" | "docx"
    ) {
        return Err(malformed());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| malformed())?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(malformed());
    }
    if text.chars().count() > MAX_TEXT {
        return Err(bad("Document exceeds one million extracted characters."));
    }
    let mime = match extension {
        "json" => {
            let _: serde_json::Value =
                serde_json::from_str(text).map_err(|_| bad("Invalid JSON document."))?;
            "application/json"
        }
        "csv" => {
            for row in csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(bytes)
                .records()
            {
                row.map_err(|_| bad("Invalid CSV document."))?;
            }
            "text/csv"
        }
        "md" | "markdown" => "text/markdown",
        "txt" | "text" | "" => "text/plain",
        _ => return Err(malformed()),
    };
    Ok(Extracted {
        content_type: mime.into(),
        text: text.into(),
        sections: vec![Section {
            offset: 0,
            label: "Document".into(),
        }],
    })
}

// quick-xml checks matching end tags, but an EOF alone does not reject an
// unclosed document or multiple roots. Enforce those document boundaries too.
#[derive(Default)]
struct XmlStructure {
    depth: usize,
    closed: bool,
}

impl XmlStructure {
    fn check(&mut self, event: &quick_xml::events::Event<'_>) -> AppResult<()> {
        use quick_xml::events::Event;
        match event {
            Event::Start(_) | Event::Empty(_) if self.closed => return Err(malformed()),
            Event::Start(_) => self.depth += 1,
            Event::Empty(_) if self.depth == 0 => self.closed = true,
            Event::End(_) => {
                self.depth = self.depth.checked_sub(1).ok_or_else(malformed)?;
                self.closed = self.depth == 0;
            }
            Event::Text(text) if self.depth == 0 && !text.iter().all(u8::is_ascii_whitespace) => {
                return Err(malformed());
            }
            Event::Eof if self.depth != 0 || !self.closed => return Err(malformed()),
            Event::DocType(_) => return Err(malformed()),
            _ => {}
        }
        Ok(())
    }
}

/// Worker protocol: extension line then raw bytes; JSON response, never parser stderr.
pub fn worker() {
    let result = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("worker runtime")
        .block_on(async {
            tokio::task::spawn_blocking(|| {
                let mut input = Vec::new();
                std::io::stdin()
                    .take((MAX_BYTES + 33) as u64)
                    .read_to_end(&mut input)
                    .map_err(|_| malformed())?;
                let split = input
                    .iter()
                    .position(|b| *b == b'\n')
                    .filter(|n| *n <= 20)
                    .ok_or_else(malformed)?;
                let extension = std::str::from_utf8(&input[..split]).map_err(|_| malformed())?;
                parse(&input[split + 1..], extension)
            })
            .await
        });
    let response: Result<Extracted, String> = match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(AppError::BadRequest(message))) => Err(message),
        _ => Err("Attachment extraction failed. Export a simpler document.".into()),
    };
    // Prefix also lets the test harness invoke the same worker without an extra binary.
    let _ = std::io::stdout().write_all(b"NYX_ATTACHMENT\n");
    let _ = serde_json::to_writer(std::io::stdout(), &response);
}

pub async fn extract(bytes: Vec<u8>, extension: String) -> AppResult<Extracted> {
    extract_deadline(bytes, extension, Duration::from_secs(8)).await
}
async fn extract_deadline(
    bytes: Vec<u8>,
    extension: String,
    deadline: Duration,
) -> AppResult<Extracted> {
    if bytes.len() > MAX_BYTES {
        return Err(bad("Attachment exceeds 20 MiB."));
    }
    let _permit = WORKERS.acquire().await.map_err(|_| malformed())?;
    let mut command =
        tokio::process::Command::new(std::env::current_exe().map_err(|_| malformed())?);
    #[cfg(not(test))]
    command.arg("--attachment-worker");
    #[cfg(test)]
    command.args([
        "--ignored",
        "--exact",
        "services::attachment_extraction::tests::attachment_worker",
        "--nocapture",
    ]);
    command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            // Only async-signal-safe syscalls between fork and exec.
            let cpu = libc::rlimit {
                rlim_cur: 6,
                rlim_max: 6,
            };
            if libc::setrlimit(libc::RLIMIT_CPU, &cpu) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            #[cfg(target_os = "linux")]
            {
                let memory = libc::rlimit {
                    rlim_cur: 1024 * 1024 * 1024,
                    rlim_max: 1024 * 1024 * 1024,
                };
                if libc::setrlimit(libc::RLIMIT_AS, &memory) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|_| bad("Attachment extraction is unavailable. Try again."))?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let operation = async {
        let send = async move {
            stdin.write_all(extension.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.write_all(&bytes).await?;
            stdin.shutdown().await?;
            drop(stdin);
            Ok::<_, std::io::Error>(())
        };
        let receive = async {
            let mut result = Vec::new();
            stdout
                .take((MAX_OUTPUT + 1) as u64)
                .read_to_end(&mut result)
                .await?;
            Ok::<_, std::io::Error>(result)
        };
        let (_, output) = tokio::try_join!(send, receive).map_err(|_| malformed())?;
        let status = child.wait().await.map_err(|_| malformed())?;
        if !status.success() || output.len() > MAX_OUTPUT {
            return Err(bad(
                "Document exceeds extraction limits. Export a smaller document.",
            ));
        }
        // Decode away from the async executor too; only bounded output crosses back.
        tokio::task::spawn_blocking(move || {
            let marker = b"NYX_ATTACHMENT\n";
            let start = output
                .windows(marker.len())
                .position(|w| w == marker)
                .ok_or_else(malformed)?
                + marker.len();
            let mut stream = serde_json::Deserializer::from_slice(&output[start..])
                .into_iter::<Result<Extracted, String>>();
            stream
                .next()
                .ok_or_else(malformed)?
                .map_err(|_| malformed())?
                .map_err(AppError::BadRequest)
        })
        .await
        .map_err(|_| malformed())?
    };
    match tokio::time::timeout(deadline, operation).await {
        Ok(result) => result,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(bad(
                "Document extraction timed out. Export a smaller or simpler document.",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "subprocess entrypoint for isolated extraction"]
    fn attachment_worker() {
        worker();
    }
    #[test]
    fn rejects_spoofed_and_oversized_files() {
        assert!(parse(b"not png", "png").is_err());
        assert!(parse(b"PK fake", "docx").is_err());
        assert!(parse(&vec![b'a'; MAX_BYTES + 1], "txt").is_err());
        assert!(parse(b"a\0b", "txt").is_err());
        assert!(parse(b"{no}", "json").is_err());
        assert!(parse(b"<svg/>", "svg").is_err());
    }
    #[tokio::test]
    async fn isolated_extraction_and_timeout_release_worker() {
        assert_eq!(
            extract(b"hello".to_vec(), "txt".into()).await.unwrap().text,
            "hello"
        );
        let error = extract_deadline(vec![b'a'; 900_000], "txt".into(), Duration::ZERO)
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("timed out"));
        assert!(
            extract(b"after timeout".to_vec(), "txt".into())
                .await
                .is_ok()
        );
    }
    #[test]
    fn encrypted_pdf_and_malformed_pdf_are_rejected() {
        use lopdf::{
            Document, EncryptionState, EncryptionVersion, Object, Permissions, dictionary,
        };
        let mut pdf = Document::with_version("1.5");
        let pages_id = pdf.new_object_id();
        let page_id = pdf.add_object(dictionary! {"Type"=>"Page","Parent"=>pages_id,"MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()]});
        pdf.objects.insert(
            pages_id,
            Object::Dictionary(
                dictionary! {"Type"=>"Pages","Kids"=>vec![page_id.into()],"Count"=>1},
            ),
        );
        let catalog = pdf.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages_id});
        pdf.trailer.set("Root", catalog);
        pdf.trailer.set(
            "ID",
            vec![
                Object::string_literal("upload-test"),
                Object::string_literal("upload-test"),
            ],
        );
        let encryption = EncryptionState::try_from(EncryptionVersion::V2 {
            document: &pdf,
            owner_password: "owner",
            user_password: "secret",
            key_length: 128,
            permissions: Permissions::empty(),
        })
        .unwrap();
        pdf.encrypt(&encryption).unwrap();
        let mut bytes = Vec::new();
        pdf.save_to(&mut bytes).unwrap();
        let error = parse(&bytes, "pdf").err().unwrap();
        assert!(error.to_string().contains("Encrypted"), "{error}");
        assert!(parse(b"%PDF-1.7 corrupt", "pdf").is_err());
    }
    #[test]
    fn docx_expansion_is_bounded_and_entities_are_not_resolved() {
        fn zip(data: &[u8]) -> Vec<u8> {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer.start_file("[Content_Types].xml", options).unwrap();
            writer.write_all(br#"<Types><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#).unwrap();
            writer.start_file("word/document.xml", options).unwrap();
            writer.write_all(data).unwrap();
            writer.finish().unwrap().into_inner()
        }
        assert!(parse(&zip(&vec![b'a'; MAX_EXPANDED as usize + 1]), "docx").is_err());
        assert!(parse(&zip(b"<!DOCTYPE a SYSTEM 'file:///private'><a/>"), "docx").is_err());
        assert!(parse(&zip(b"<w:document><w:p><w:t>unclosed"), "docx").is_err());
        assert!(parse(&zip(b"<w:document></w:document><w:document/>"), "docx").is_err());
        assert_eq!(
            parse(
                &zip(b"<w:document><w:p><w:t>Hello &amp; bye</w:t></w:p></w:document>"),
                "docx"
            )
            .unwrap()
            .text,
            "Hello & bye\n"
        );
    }
}
