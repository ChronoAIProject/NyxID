#!/usr/bin/env python3
"""Render a synthetic rotated PDF through the backend's packaged runtime layer.

No private files, credentials, host converter, Python packages or PDF libraries.
Run on the native amd64/arm64 Publish Images runners before the backend build.
"""
import argparse
import json
import struct
import subprocess


def synthetic_pdf():
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Rotate 90 /Contents 4 0 R /Resources << >> >>",
    ]
    stream = b"0.25 0.5 0.75 rg 20 20 200 150 re f\n"
    objects.append(b"<< /Length " + str(len(stream)).encode() + b" >>\nstream\n" + stream + b"endstream")
    output = b"%PDF-1.5\n"
    offsets = []
    for index, value in enumerate(objects, 1):
        offsets.append(len(output))
        output += f"{index} 0 obj\n".encode() + value + b"\nendobj\n"
    xref = len(output)
    output += f"xref\n0 {len(objects)+1}\n0000000000 65535 f \n".encode()
    for offset in offsets:
        output += f"{offset:010d} 00000 n \n".encode()
    return output + f"trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()


def jpeg_size(data):
    assert data[:2] == b"\xff\xd8", "not a JPEG"
    offset = 2
    while offset < len(data):
        assert data[offset] == 255
        marker = data[offset + 1]
        length = struct.unpack(">H", data[offset + 2:offset + 4])[0]
        if marker in (192, 193, 194):
            height, width = struct.unpack(">HH", data[offset + 5:offset + 9])
            return width, height
        offset += 2 + length
    raise AssertionError("missing JPEG dimensions")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--context")
    parser.add_argument("--platform", required=True, choices=["linux/amd64", "linux/arm64"])
    args = parser.parse_args()
    docker = ["docker"] + (["--context", args.context] if args.context else [])
    tag = "nyxid-pdf-renderer-check:" + args.platform.split("/")[1]
    subprocess.run(docker + ["build", "--pull", "--platform", args.platform, "--target", "pdf-runtime", "-f", "backend/Dockerfile", "-t", tag, "."], check=True)
    architecture = subprocess.check_output(docker + ["image", "inspect", tag, "--format", "{{.Architecture}}"], text=True).strip()
    assert architecture == args.platform.split("/")[1], architecture
    subprocess.run(docker + ["run", "--rm", "--platform", args.platform, tag, "/usr/bin/pdftoppm", "-v"], check=True)
    result = subprocess.run(docker + ["run", "--rm", "-i", "--platform", args.platform, "--user", "65534:65534", "--network", "none", "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", tag, "/bin/sh", "-c", "ulimit -v 131072; ulimit -t 6; exec /usr/bin/pdftoppm -f 1 -l 1 -singlefile -scale-to 2048 -jpeg -jpegopt quality=85 -"], input=synthetic_pdf(), stdout=subprocess.PIPE, check=True, timeout=20)
    size = jpeg_size(result.stdout)
    assert size == (2048, 1536), size
    assert len(result.stdout) <= 2 * 1024 * 1024
    print(json.dumps({"platform": args.platform, "dimensions": size, "encoded_bytes": len(result.stdout), "result": "passed"}))


if __name__ == "__main__":
    main()
