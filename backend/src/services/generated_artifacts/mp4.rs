//! Bounded ISO-BMFF walk. Codec markers outside a real visual track prove nothing.
use super::sandbox::Checked;
const INVALID: &str = "artifact_invalid";
const MAX_BOXES: usize = 4096;
const MAX_TRACKS: usize = 128;
const MAX_SAMPLES: usize = 100_000;
const MAX_TABLE_ENTRIES: usize = 200_000;
const MAX_WORK: usize = 1_000_000;

#[derive(Default)]
struct Work {
    boxes: usize,
    samples: usize,
    tables: usize,
    steps: usize,
}
impl Work {
    fn charge(&mut self, steps: usize) -> Checked<()> {
        self.steps = self
            .steps
            .checked_add(steps)
            .filter(|n| *n <= MAX_WORK)
            .ok_or(INVALID)?;
        Ok(())
    }
    fn samples(&mut self, n: usize) -> Checked<()> {
        self.samples = self
            .samples
            .checked_add(n)
            .filter(|n| *n <= MAX_SAMPLES)
            .ok_or(INVALID)?;
        self.charge(n)
    }
}

struct Media(Vec<(u64, u64)>);
impl Media {
    fn new(top: &[BoxRef<'_>], data: &[u8], work: &mut Work) -> Checked<Self> {
        let mut ranges = Vec::new();
        for b in top.iter().filter(|b| b.kind == *b"mdat") {
            work.charge(1)?;
            let start = (b.data.as_ptr() as usize)
                .checked_sub(data.as_ptr() as usize)
                .ok_or(INVALID)? as u64;
            let end = start.checked_add(b.data.len() as u64).ok_or(INVALID)?;
            ranges.push((start, end));
        }
        ranges.sort_unstable();
        if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err(INVALID);
        }
        Ok(Self(ranges))
    }
    fn contains(&self, offset: u64, size: u64, work: &mut Work) -> Checked<bool> {
        let end = offset.checked_add(size).ok_or(INVALID)?;
        let (mut lo, mut hi) = (0, self.0.len());
        while lo < hi {
            work.charge(1)?;
            let mid = lo + (hi - lo) / 2;
            if self.0[mid].0 <= offset {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Ok(lo > 0 && offset >= self.0[lo - 1].0 && end <= self.0[lo - 1].1)
    }
}

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
fn boxes<'a>(data: &'a [u8], work: &mut Work) -> Checked<Vec<BoxRef<'a>>> {
    let mut output = Vec::new();
    let mut offset = 0usize;
    while offset < data.len() {
        work.charge(1)?;
        work.boxes = work.boxes.checked_add(1).ok_or(INVALID)?;
        if work.boxes > MAX_BOXES {
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
fn table<'a>(data: &'a [u8], stride: usize, work: &mut Work) -> Checked<&'a [u8]> {
    let n = u32_at(data, 4)? as usize;
    work.tables = work
        .tables
        .checked_add(n)
        .filter(|n| *n <= MAX_TABLE_ENTRIES)
        .ok_or(INVALID)?;
    work.charge(n)?;
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
/// Validate sample counts, chunk mappings and complete sample extents. Merely
/// pointing a chunk offset somewhere inside mdat is insufficient.
fn samples(
    stbl: &[BoxRef<'_>],
    media: &Media,
    descriptions: usize,
    work: &mut Work,
) -> Checked<()> {
    let stsz = one(stbl, b"stsz")?.data;
    let fixed = u32_at(stsz, 4)?;
    let count = u32_at(stsz, 8)? as usize;
    work.samples(count)?;
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
    let timing = table(one(stbl, b"stts")?.data, 8, work)?;
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
    let mapping = table(one(stbl, b"stsc")?.data, 12, work)?;
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
    let offsets = table(offsets.data, stride, work)?;
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
        if !media.contains(offset, size, work)? {
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
    media: &Media,
    work: &mut Work,
) -> Checked<()> {
    let mvex = boxes(one(moov, b"mvex")?.data, work)?;
    let mut defaults = Vec::new();
    for entry in mvex.iter().filter(|b| b.kind == *b"trex") {
        if entry.data.len() != 24 || u32_at(entry.data, 0)? != 0 {
            return Err(INVALID);
        }
        let id = u32_at(entry.data, 4)?;
        if id == 0 || defaults.iter().any(|(existing, _, _, _)| *existing == id) {
            return Err(INVALID);
        }
        if defaults.len() >= MAX_TRACKS {
            return Err(INVALID);
        }
        work.charge(defaults.len() + 1)?;
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
        let children = boxes(moof.data, work)?;
        let mfhd = one(&children, b"mfhd")?.data;
        if mfhd.len() != 8 || u32_at(mfhd, 0)? != 0 {
            return Err(INVALID);
        }
        let mut traf_count = 0;
        for traf in children.iter().filter(|b| b.kind == *b"traf") {
            traf_count += 1;
            let entries = boxes(traf.data, work)?;
            let tfhd = one(&entries, b"tfhd")?.data;
            let flags = u32_at(tfhd, 0)?;
            if flags & !0x02003b != 0 {
                return Err(INVALID);
            }
            let id = u32_at(tfhd, 4)?;
            work.charge(tracks.len() + defaults.len())?;
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
                work.samples(n)?;
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
                        || !media.contains(position, u64::from(sample_size), work)?
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
    let mut work = Work::default();
    validate_with_work(data, &mut work)
}
fn validate_with_work(data: &[u8], work: &mut Work) -> Checked<bool> {
    let top = boxes(data, work)?;
    let media = Media::new(&top, data, work)?;
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
    let moov = boxes(one(&top, b"moov")?.data, work)?;
    let fragmented =
        moov.iter().any(|b| b.kind == *b"mvex") || top.iter().any(|b| b.kind == *b"moof");
    let mut tracks = Vec::new();
    let mut visual = false;
    let mut preview = false;
    for trak in moov.iter().filter(|b| b.kind == *b"trak") {
        let track = boxes(trak.data, work)?;
        let tkhd = one(&track, b"tkhd")?.data;
        let version = *tkhd.first().ok_or(INVALID)?;
        let dim = match version {
            0 => 76,
            1 => 88,
            _ => return Err(INVALID),
        };
        let width = u32_at(tkhd, dim)? >> 16;
        let height = u32_at(tkhd, dim + 4)? >> 16;
        let mdia = boxes(one(&track, b"mdia")?.data, work)?;
        let handler = one(&mdia, b"hdlr")?.data.get(8..12).ok_or(INVALID)?;
        let minf = boxes(one(&mdia, b"minf")?.data, work)?;
        let stbl = boxes(one(&minf, b"stbl")?.data, work)?;
        let stsd = one(&stbl, b"stsd")?.data;
        let entries = boxes(stsd.get(8..).ok_or(INVALID)?, work)?;
        if u32_at(stsd, 4)? as usize != entries.len() || entries.is_empty() {
            return Err(INVALID);
        }
        if tracks.len() >= MAX_TRACKS {
            return Err(INVALID);
        }
        work.charge(tracks.len() + 1)?;
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
                samples(&stbl, &media, entries.len(), work)?;
            }
        }
        if handler != b"vide" {
            continue;
        }
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(INVALID);
        }
        let dinf = boxes(one(&minf, b"dinf")?.data, work)?;
        let dref = one(&dinf, b"dref")?.data;
        let references = boxes(dref.get(8..).ok_or(INVALID)?, work)?;
        if u32_at(dref, 4)? != 1
            || references.len() != 1
            || references[0].kind != *b"url "
            || references[0].data != [0, 0, 0, 1]
        {
            return Err(INVALID);
        }
        if !fragmented {
            samples(&stbl, &media, entries.len(), work)?;
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
            let configs = boxes(&entry.data[78..], work)?;
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
        fragments(data, &top, &moov, &tracks, &media, work)?;
        return Ok(false);
    }
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    const MP4: &[u8] = include_bytes!("../../../tests/fixtures/generated-artifacts/visual-avc.mp4");
    fn atom(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend(kind);
        out.extend(payload);
        out
    }
    fn child(data: &[u8], kind: &[u8; 4]) -> Vec<u8> {
        one(&boxes(data, &mut Work::default()).unwrap(), kind)
            .unwrap()
            .data
            .to_vec()
    }
    fn replace(data: &[u8], path: &[[u8; 4]], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for b in boxes(data, &mut Work::default()).unwrap() {
            let value = if b.kind == path[0] {
                if path.len() == 1 {
                    payload.to_vec()
                } else {
                    replace(b.data, &path[1..], payload)
                }
            } else {
                b.data.to_vec()
            };
            out.extend(atom(&b.kind, &value));
        }
        out
    }
    #[test]
    fn real_encoder_fixtures_keep_preview_and_download_distinct() {
        assert_eq!(validate(MP4), Ok(true));
        assert_eq!(
            validate(include_bytes!(
                "../../../tests/fixtures/generated-artifacts/visual-high.mp4"
            )),
            Ok(true)
        );
        assert_eq!(
            validate(include_bytes!(
                "../../../tests/fixtures/generated-artifacts/visual-fragmented.mp4"
            )),
            Ok(false)
        );
        assert!(validate(b"ftypisommoovtrakmdiahdlrvidestblstsdavc1").is_err());
    }
    #[test]
    fn many_media_boxes_and_tracks_have_a_whole_file_work_bound() {
        // Every chunk legally references the last tiny mdat. The old validator
        // rescanned 3,000 media boxes for each of 800,000 chunk offsets.
        let ftyp = atom(b"ftyp", &child(MP4, b"ftyp"));
        let mut file = ftyp;
        for _ in 0..3000 {
            file.extend(atom(b"mdat", &[1]));
        }
        let offset = (file.len() - 1) as u32;
        let moov = child(MP4, b"moov");
        let mut track = child(&moov, b"trak");
        let mut offsets = vec![0; 4];
        offsets.extend(100_000u32.to_be_bytes());
        for _ in 0..100_000 {
            offsets.extend(offset.to_be_bytes());
        }
        let mut sizes = vec![0; 4];
        sizes.extend(1u32.to_be_bytes());
        sizes.extend(100_000u32.to_be_bytes());
        let timing = [0u32, 1, 100_000, 1]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect::<Vec<_>>();
        let mapping = [0u32, 1, 1, 1, 1]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect::<Vec<_>>();
        for (kind, table) in [
            (*b"stco", offsets),
            (*b"stsz", sizes),
            (*b"stts", timing),
            (*b"stsc", mapping),
        ] {
            track = replace(&track, &[*b"mdia", *b"minf", *b"stbl", kind], &table);
        }
        let mut movie = Vec::new();
        for b in boxes(&moov, &mut Work::default()).unwrap() {
            if b.kind != *b"trak" {
                movie.extend(atom(&b.kind, b.data));
            }
        }
        for id in 1u32..=8 {
            let mut header = child(&track, b"tkhd");
            header[12..16].copy_from_slice(&id.to_be_bytes());
            movie.extend(atom(b"trak", &replace(&track, &[*b"tkhd"], &header)));
        }
        file.extend(atom(b"moov", &movie));
        assert!(file.len() < 8 * 1024 * 1024);
        let start = std::time::Instant::now();
        let mut work = Work::default();
        assert!(validate_with_work(&file, &mut work).is_err());
        assert!(work.steps <= MAX_WORK);
        assert!(work.samples <= MAX_SAMPLES);
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }
    #[test]
    fn media_membership_uses_logarithmic_probes_and_checked_extents() {
        let media = Media((0..3700).map(|n| (n * 16, n * 16 + 8)).collect());
        let mut work = Work::default();
        for _ in 0..10_000 {
            assert!(media.contains(3699 * 16, 8, &mut work).unwrap());
        }
        assert!(work.steps <= 130_000);
        assert!(!media.contains(3699 * 16, 9, &mut work).unwrap());
        assert!(media.contains(u64::MAX, 1, &mut work).is_err());
    }
}
