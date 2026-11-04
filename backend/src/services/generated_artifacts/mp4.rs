//! Bounded ISO-BMFF walk. Codec markers outside a real visual track prove nothing.
use super::sandbox::Checked;
const INVALID: &str = "artifact_invalid";
#[derive(Clone, Copy)]
struct BoxRef<'a> {
    kind: [u8; 4],
    data: &'a [u8],
    header: usize,
}
fn u32_at(data: &[u8], offset: usize) -> Checked<u32> {
    Ok(u32::from_be_bytes(
        data.get(offset..offset.checked_add(4).ok_or(INVALID)?)
            .ok_or(INVALID)?
            .try_into()
            .map_err(|_| INVALID)?,
    ))
}
fn boxes<'a>(data: &'a [u8], count: &mut usize) -> Checked<Vec<BoxRef<'a>>> {
    let mut output = Vec::new();
    let mut offset = 0usize;
    while offset < data.len() {
        *count = count.checked_add(1).ok_or(INVALID)?;
        if *count > 4096 {
            return Err(INVALID);
        }
        let size = u32_at(data, offset)?;
        let kind = data
            .get(offset + 4..offset + 8)
            .ok_or(INVALID)?
            .try_into()
            .map_err(|_| INVALID)?;
        let (size, header) = match size {
            0 => (data.len() - offset, 8),
            1 => {
                let n = u64::from_be_bytes(
                    data.get(offset + 8..offset + 16)
                        .ok_or(INVALID)?
                        .try_into()
                        .map_err(|_| INVALID)?,
                );
                (usize::try_from(n).map_err(|_| INVALID)?, 16)
            }
            n => (n as usize, 8),
        };
        let end = offset.checked_add(size).ok_or(INVALID)?;
        if size < header || end > data.len() {
            return Err(INVALID);
        }
        output.push(BoxRef {
            kind,
            header,
            data: &data[offset + header..end],
        });
        offset = end;
    }
    Ok(output)
}
fn one<'a>(all: &[BoxRef<'a>], kind: &[u8; 4]) -> Checked<BoxRef<'a>> {
    let mut found = all.iter().filter(|b| &b.kind == kind);
    let first = *found.next().ok_or(INVALID)?;
    if found.next().is_some() {
        return Err(INVALID);
    }
    Ok(first)
}
fn table(data: &[u8], stride: usize) -> Checked<&[u8]> {
    let n = u32_at(data, 4)? as usize;
    if n == 0
        || n > 100_000
        || data.len()
            != 8usize
                .checked_add(n.checked_mul(stride).ok_or(INVALID)?)
                .ok_or(INVALID)?
    {
        return Err(INVALID);
    }
    data.get(8..).ok_or(INVALID)
}
fn in_media(top: &[BoxRef<'_>], data: &[u8], offset: u64, size: u64) -> bool {
    top.iter().filter(|b| b.kind == *b"mdat").any(|b| {
        let start = (b.data.as_ptr() as usize - data.as_ptr() as usize) as u64;
        offset >= start
            && offset
                .checked_add(size)
                .is_some_and(|end| end <= start + b.data.len() as u64)
    })
}
/// Validate sample counts, chunk mappings and complete sample extents. Merely
/// pointing a chunk offset somewhere inside mdat is insufficient.
fn samples(
    stbl: &[BoxRef<'_>],
    top: &[BoxRef<'_>],
    data: &[u8],
    descriptions: usize,
) -> Checked<()> {
    let stsz = one(stbl, b"stsz")?.data;
    let fixed = u32_at(stsz, 4)?;
    let count = u32_at(stsz, 8)? as usize;
    if count == 0 || count > 100_000 {
        return Err(INVALID);
    }
    let expected = if fixed == 0 {
        12usize
            .checked_add(count.checked_mul(4).ok_or(INVALID)?)
            .ok_or(INVALID)?
    } else {
        12
    };
    if stsz.len() != expected {
        return Err(INVALID);
    }
    let timing = table(one(stbl, b"stts")?.data, 8)?;
    let mut timed = 0usize;
    for entry in timing.as_chunks::<8>().0 {
        let n = u32_at(entry, 0)? as usize;
        if n == 0 || u32_at(entry, 4)? == 0 {
            return Err(INVALID);
        }
        timed = timed.checked_add(n).ok_or(INVALID)?;
    }
    if timed != count {
        return Err(INVALID);
    }
    let mapping = table(one(stbl, b"stsc")?.data, 12)?;
    let mut chunks = Vec::new();
    let mut previous = 0;
    for entry in mapping.as_chunks::<12>().0 {
        let first = u32_at(entry, 0)? as usize;
        let n = u32_at(entry, 4)? as usize;
        let description = u32_at(entry, 8)? as usize;
        if first <= previous
            || n == 0
            || n > count
            || description == 0
            || description > descriptions
        {
            return Err(INVALID);
        }
        chunks.push((first, n));
        previous = first;
    }
    if chunks[0].0 != 1 {
        return Err(INVALID);
    }
    let mut offset_tables = stbl.iter().filter(|b| matches!(&b.kind, b"stco" | b"co64"));
    let offsets = offset_tables.next().ok_or(INVALID)?;
    if offset_tables.next().is_some() {
        return Err(INVALID);
    }
    let stride = if offsets.kind == *b"stco" { 4 } else { 8 };
    let offsets = table(offsets.data, stride)?;
    if chunks.last().ok_or(INVALID)?.0 > offsets.len() / stride {
        return Err(INVALID);
    }
    let mut mapping_index = 0;
    let mut sample = 0usize;
    for (index, raw) in offsets.chunks_exact(stride).enumerate() {
        while mapping_index + 1 < chunks.len() && chunks[mapping_index + 1].0 <= index + 1 {
            mapping_index += 1;
        }
        let end_sample = sample
            .checked_add(chunks[mapping_index].1)
            .filter(|end| *end <= count)
            .ok_or(INVALID)?;
        let mut size = 0u64;
        while sample < end_sample {
            let n = if fixed == 0 {
                u32_at(stsz, 12 + sample * 4)?
            } else {
                fixed
            };
            if n == 0 {
                return Err(INVALID);
            }
            size = size.checked_add(u64::from(n)).ok_or(INVALID)?;
            sample += 1;
        }
        let offset = if stride == 4 {
            u64::from(u32_at(raw, 0)?)
        } else {
            u64::from_be_bytes(raw.try_into().map_err(|_| INVALID)?)
        };
        if !in_media(top, data, offset, size) {
            return Err(INVALID);
        }
    }
    if sample != count {
        return Err(INVALID);
    }
    Ok(())
}
fn avc_preview(config: &[u8]) -> Checked<bool> {
    if config.len() < 7 || config[0] != 1 || config[4] & 0xfc != 0xfc || config[5] & 0xe0 != 0xe0 {
        return Err(INVALID);
    }
    let mut offset = 6usize;
    let sps = config[5] & 0x1f;
    if sps == 0 {
        return Err(INVALID);
    }
    for (count, nal_type) in [(sps, 7u8), (0, 8)] {
        let count = if nal_type == 8 {
            let n = *config.get(offset).ok_or(INVALID)?;
            offset += 1;
            if n == 0 {
                return Err(INVALID);
            }
            n
        } else {
            count
        };
        for _ in 0..count {
            let len = u16::from_be_bytes(
                config
                    .get(offset..offset + 2)
                    .ok_or(INVALID)?
                    .try_into()
                    .map_err(|_| INVALID)?,
            ) as usize;
            offset += 2;
            let end = offset.checked_add(len).ok_or(INVALID)?;
            let nal = config.get(offset..end).ok_or(INVALID)?;
            if nal.is_empty() || nal[0] & 0x1f != nal_type {
                return Err(INVALID);
            }
            if nal_type == 7 && (nal.len() < 4 || nal[1..4] != config[1..4]) {
                return Err(INVALID);
            }
            offset = end;
        }
    }
    let mut high_420 = false;
    if matches!(config[1], 100 | 110 | 122 | 144) && offset < config.len() {
        let extension = config.get(offset..offset + 4).ok_or(INVALID)?;
        if extension[0] & 0xfc != 0xfc || extension[1] & 0xf8 != 0xf8 || extension[2] & 0xf8 != 0xf8
        {
            return Err(INVALID);
        }
        high_420 = extension[..3] == [0xfd, 0xf8, 0xf8];
        offset += 4;
        for _ in 0..extension[3] {
            let len = u16::from_be_bytes(
                config
                    .get(offset..offset + 2)
                    .ok_or(INVALID)?
                    .try_into()
                    .map_err(|_| INVALID)?,
            ) as usize;
            offset += 2;
            let end = offset.checked_add(len).ok_or(INVALID)?;
            let nal = config.get(offset..end).ok_or(INVALID)?;
            if nal.is_empty() || nal[0] & 0x1f != 13 {
                return Err(INVALID);
            }
            offset = end;
        }
    }
    if offset != config.len() {
        return Err(INVALID);
    }
    // Baseline and eight-bit 4:2:0 High use real encoder fixtures. The browser
    // remains the final decoder and retains the same-byte download fallback.
    Ok((config[1] == 66 || (config[1] == 100 && high_420)) && config[4] & 3 == 3)
}
#[derive(Clone, Copy)]
struct FragmentTrack {
    id: u32,
    descriptions: usize,
    visual: bool,
}
fn empty_samples(stbl: &[BoxRef<'_>]) -> Checked<()> {
    if one(stbl, b"stsz")?.data != [0; 12] {
        return Err(INVALID);
    }
    for kind in [b"stts", b"stsc"] {
        if one(stbl, kind)?.data != [0; 8] {
            return Err(INVALID);
        }
    }
    let mut offsets = stbl.iter().filter(|b| matches!(&b.kind, b"stco" | b"co64"));
    if offsets.next().ok_or(INVALID)?.data != [0; 8] || offsets.next().is_some() {
        return Err(INVALID);
    }
    Ok(())
}
/// Fragmented files are download-only. Validate actual sample extents through
/// trex/tfhd/trun, including signed offsets and every declared field boundary.
fn fragments(
    data: &[u8],
    top: &[BoxRef<'_>],
    moov: &[BoxRef<'_>],
    tracks: &[FragmentTrack],
    count: &mut usize,
) -> Checked<()> {
    let mvex = boxes(one(moov, b"mvex")?.data, count)?;
    let mut defaults = Vec::new();
    for entry in mvex.iter().filter(|b| b.kind == *b"trex") {
        if entry.data.len() != 24 || u32_at(entry.data, 0)? != 0 {
            return Err(INVALID);
        }
        let id = u32_at(entry.data, 4)?;
        if id == 0 || defaults.iter().any(|(existing, _, _, _)| *existing == id) {
            return Err(INVALID);
        }
        defaults.push((
            id,
            u32_at(entry.data, 8)?,
            u32_at(entry.data, 12)?,
            u32_at(entry.data, 16)?,
        ));
    }
    let mut total = 0usize;
    let mut visual = false;
    for moof in top.iter().filter(|b| b.kind == *b"moof") {
        let start = (moof.data.as_ptr() as usize)
            .checked_sub(data.as_ptr() as usize)
            .and_then(|n| n.checked_sub(moof.header))
            .ok_or(INVALID)? as u64;
        let children = boxes(moof.data, count)?;
        let mfhd = one(&children, b"mfhd")?.data;
        if mfhd.len() != 8 || u32_at(mfhd, 0)? != 0 {
            return Err(INVALID);
        }
        let mut traf_count = 0;
        for traf in children.iter().filter(|b| b.kind == *b"traf") {
            traf_count += 1;
            let entries = boxes(traf.data, count)?;
            let tfhd = one(&entries, b"tfhd")?.data;
            let flags = u32_at(tfhd, 0)?;
            if flags & !0x02003b != 0 {
                return Err(INVALID);
            }
            let id = u32_at(tfhd, 4)?;
            let track = tracks.iter().find(|t| t.id == id).ok_or(INVALID)?;
            let (_, mut description, mut duration, mut size) = *defaults
                .iter()
                .find(|(t, _, _, _)| *t == id)
                .ok_or(INVALID)?;
            let mut offset = 8usize;
            let base = if flags & 1 != 0 {
                let n = u64::from_be_bytes(
                    tfhd.get(offset..offset + 8)
                        .ok_or(INVALID)?
                        .try_into()
                        .map_err(|_| INVALID)?,
                );
                offset += 8;
                n
            } else if flags & 0x020000 != 0 {
                start
            } else {
                return Err(INVALID);
            };
            for (flag, value) in [(2, &mut description), (8, &mut duration), (16, &mut size)] {
                if flags & flag != 0 {
                    *value = u32_at(tfhd, offset)?;
                    offset += 4;
                }
            }
            if flags & 32 != 0 {
                u32_at(tfhd, offset)?;
                offset += 4;
            }
            if offset != tfhd.len() || description == 0 || description as usize > track.descriptions
            {
                return Err(INVALID);
            }
            if let Some(tfdt) = entries.iter().find(|b| b.kind == *b"tfdt") {
                let version = u32_at(tfdt.data, 0)?;
                if !matches!((version, tfdt.data.len()), (0, 8) | (0x01000000, 12)) {
                    return Err(INVALID);
                }
                one(&entries, b"tfdt")?;
            }
            let mut previous_end = None;
            let mut runs = 0;
            for run in entries.iter().filter(|b| b.kind == *b"trun") {
                runs += 1;
                let flags = u32_at(run.data, 0)?;
                if flags & !0x01000f05 != 0
                    || flags >> 24 > 1
                    || (flags & 4 != 0 && flags & 0x400 != 0)
                {
                    return Err(INVALID);
                }
                let n = u32_at(run.data, 4)? as usize;
                total = total
                    .checked_add(n)
                    .filter(|n| *n <= 100_000)
                    .ok_or(INVALID)?;
                if n == 0 {
                    return Err(INVALID);
                }
                let mut offset = 8usize;
                let mut position = if flags & 1 != 0 {
                    let delta = u32_at(run.data, offset)? as i32;
                    offset += 4;
                    base.checked_add_signed(i64::from(delta)).ok_or(INVALID)?
                } else {
                    previous_end.unwrap_or(base)
                };
                if flags & 4 != 0 {
                    u32_at(run.data, offset)?;
                    offset += 4;
                }
                let stride = [0x100, 0x200, 0x400, 0x800]
                    .iter()
                    .filter(|flag| flags & **flag != 0)
                    .count()
                    * 4;
                if offset
                    .checked_add(n.checked_mul(stride).ok_or(INVALID)?)
                    .ok_or(INVALID)?
                    != run.data.len()
                {
                    return Err(INVALID);
                }
                for _ in 0..n {
                    let sample_duration = if flags & 0x100 != 0 {
                        let value = u32_at(run.data, offset)?;
                        offset += 4;
                        value
                    } else {
                        duration
                    };
                    let sample_size = if flags & 0x200 != 0 {
                        let value = u32_at(run.data, offset)?;
                        offset += 4;
                        value
                    } else {
                        size
                    };
                    for flag in [0x400, 0x800] {
                        if flags & flag != 0 {
                            u32_at(run.data, offset)?;
                            offset += 4;
                        }
                    }
                    if sample_duration == 0
                        || sample_size == 0
                        || !in_media(top, data, position, u64::from(sample_size))
                    {
                        return Err(INVALID);
                    }
                    position = position
                        .checked_add(u64::from(sample_size))
                        .ok_or(INVALID)?;
                }
                previous_end = Some(position);
                visual |= track.visual;
            }
            if runs == 0 {
                return Err(INVALID);
            }
        }
        if traf_count == 0 {
            return Err(INVALID);
        }
    }
    if !visual || total == 0 {
        return Err(INVALID);
    }
    Ok(())
}
/// Only AVC configurations tested in our browser fixture enable preview. Other
/// allowlisted visual entries stay downloadable; the browser is the final decoder.
pub fn validate(data: &[u8]) -> Checked<bool> {
    let mut count = 0;
    let top = boxes(data, &mut count)?;
    let ftyp = one(&top, b"ftyp")?;
    if ftyp.data.len() < 8 || ftyp.data.len() % 4 != 0 {
        return Err(INVALID);
    }
    if !ftyp
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .any(|(i, b)| {
            i != 1 && matches!(b, b"isom" | b"iso2" | b"mp41" | b"mp42" | b"avc1" | b"M4V ")
        })
    {
        return Err(INVALID);
    }
    if !top.iter().any(|b| b.kind == *b"mdat" && !b.data.is_empty()) {
        return Err(INVALID);
    }
    let moov = boxes(one(&top, b"moov")?.data, &mut count)?;
    let fragmented =
        moov.iter().any(|b| b.kind == *b"mvex") || top.iter().any(|b| b.kind == *b"moof");
    let mut tracks = Vec::new();
    let mut visual = false;
    let mut preview = false;
    for trak in moov.iter().filter(|b| b.kind == *b"trak") {
        let track = boxes(trak.data, &mut count)?;
        let tkhd = one(&track, b"tkhd")?.data;
        let version = *tkhd.first().ok_or(INVALID)?;
        let dim = match version {
            0 => 76,
            1 => 88,
            _ => return Err(INVALID),
        };
        let width = u32_at(tkhd, dim)? >> 16;
        let height = u32_at(tkhd, dim + 4)? >> 16;
        let mdia = boxes(one(&track, b"mdia")?.data, &mut count)?;
        let handler = one(&mdia, b"hdlr")?.data.get(8..12).ok_or(INVALID)?;
        let minf = boxes(one(&mdia, b"minf")?.data, &mut count)?;
        let stbl = boxes(one(&minf, b"stbl")?.data, &mut count)?;
        let stsd = one(&stbl, b"stsd")?.data;
        let entries = boxes(stsd.get(8..).ok_or(INVALID)?, &mut count)?;
        if u32_at(stsd, 4)? as usize != entries.len() || entries.is_empty() {
            return Err(INVALID);
        }
        let id = u32_at(tkhd, if version == 0 { 12 } else { 20 })?;
        if id == 0 || tracks.iter().any(|t: &FragmentTrack| t.id == id) {
            return Err(INVALID);
        }
        tracks.push(FragmentTrack {
            id,
            descriptions: entries.len(),
            visual: handler == b"vide",
        });
        if fragmented {
            if u32_at(one(&stbl, b"stsz")?.data, 8)? == 0 {
                empty_samples(&stbl)?;
            } else {
                // Fragmented containers may retain genuine initial samples.
                samples(&stbl, &top, data, entries.len())?;
            }
        }
        if handler != b"vide" {
            continue;
        }
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(INVALID);
        }
        let dinf = boxes(one(&minf, b"dinf")?.data, &mut count)?;
        let dref = one(&dinf, b"dref")?.data;
        let references = boxes(dref.get(8..).ok_or(INVALID)?, &mut count)?;
        if u32_at(dref, 4)? != 1
            || references.len() != 1
            || references[0].kind != *b"url "
            || references[0].data != [0, 0, 0, 1]
        {
            return Err(INVALID);
        }
        if !fragmented {
            samples(&stbl, &top, data, entries.len())?;
        }
        for entry in entries {
            if !matches!(&entry.kind, b"avc1" | b"avc3" | b"hvc1" | b"hev1" | b"av01") {
                return Err(INVALID);
            }
            if entry.data.len() < 78
                || entry.data[6..8] != [0, 1]
                || u16::from_be_bytes(entry.data[24..26].try_into().map_err(|_| INVALID)?) == 0
                || u16::from_be_bytes(entry.data[26..28].try_into().map_err(|_| INVALID)?) == 0
            {
                return Err(INVALID);
            }
            let configs = boxes(&entry.data[78..], &mut count)?;
            let config = one(
                &configs,
                match &entry.kind {
                    b"avc1" | b"avc3" => b"avcC",
                    b"hvc1" | b"hev1" => b"hvcC",
                    _ => b"av1C",
                },
            )?;
            if config.data.len() < 7 {
                return Err(INVALID);
            }
            visual = true;
            if matches!(&entry.kind, b"avc1" | b"avc3") {
                preview |= avc_preview(config.data)?;
            }
        }
    }
    if !visual {
        return Err(INVALID);
    }
    if fragmented {
        fragments(data, &top, &moov, &tracks, &mut count)?;
        return Ok(false);
    }
    Ok(preview)
}
