//! Independent source bytes and Python hashlib digests; shared by real-reader
//! and MCP-flow regressions. Never derive the expected bytes from a response.
use serde_json::Value;
use std::io::{Cursor, Write};

fn fill(size: usize, header: &str, unit: &str) -> Vec<u8> {
    let mut bytes = header.as_bytes().to_vec();
    while bytes.len() + unit.len() <= size {
        bytes.extend_from_slice(unit.as_bytes());
    }
    bytes.resize(size, b'x');
    bytes
}

pub(crate) fn files() -> Vec<(&'static str, Vec<u8>)> {
    let label = serde_json::to_string(&"中文 JSON 🙂\n\"\\\t".repeat(320)).unwrap();
    let size = format!("{{\"complete\":false,\"padding\":\"\",\"说明\":{label}}}").len();
    let padding = "x".repeat(12001 - size);
    let config = format!("{{\"complete\":false,\"padding\":\"{padding}\",\"说明\":{label}}}");
    vec![
        (
            "SKILL.md",
            fill(
                16001,
                "---\nname: source-recovery\ndescription: 合成资料完整读取\n---\n# 安装流程\n",
                "中文步骤：读取来源，检查结果🙂。\n",
            ),
        ),
        ("references/config.json", config.into_bytes()),
        (
            "references/escaping.txt",
            fill(
                8000,
                "Escaped controls (synthetic text):\n",
                "中文🙂\"\\\n\r\t\u{0001}\u{0008}\u{000c}",
            ),
        ),
        (
            "references/final.md",
            fill(
                4149,
                "# 最后检查\n",
                "独立读回私有 pin 和操作范围；失败就报告未完成。\n",
            ),
        ),
        (
            "references/步骤.md",
            fill(9001, "# 五文件来源\n", "检查 UTF-8 边界：é、汉字、🙂。\n"),
        ),
    ]
}

pub(crate) fn expected() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/skill-read-source.sha256.json"
    ))
    .unwrap()
}

pub(crate) fn archive() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, bytes) in files() {
        zip.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
