//! Pure-Rust PNG cover decoder (using `miniz_oxide`) and QR Code encoder for TrinityXSX.
//! Zero external image/QR crates, zero Wine GDI+ dependencies.

use std::fs;
use std::path::Path;

/// Decoded image stored as bottom-up 32-bit BGRA pixels (ready for Win32 `StretchDIBits`).
#[derive(Debug, Clone)]
pub struct DibImage {
    pub width: u32,
    pub height: u32,
    pub bgra_bottom_up: Vec<u8>,
}

const MAX_PNG_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_IMAGE_DIM: u32 = 4096;

fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i32;
    let b = b as i32;
    let c = c as i32;
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

/// Decode a standard 8-bit non-interlaced PNG into a bottom-up BGRA DIB pre-blended onto `bg_rgb`.
pub fn decode_png(bytes: &[u8], bg_rgb: (u8, u8, u8)) -> Result<DibImage, &'static str> {
    const SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 33 || &bytes[..8] != SIG {
        return Err("not a PNG");
    }

    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut color_type = 0u8;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat = Vec::new();
    let mut saw_ihdr = false;

    while pos + 12 <= bytes.len() {
        let len = be_u32(&bytes[pos..pos + 4]) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let data_start = pos + 8;
        let data_end = data_start.checked_add(len).ok_or("chunk overflow")?;
        if data_end + 4 > bytes.len() {
            return Err("truncated PNG chunk");
        }
        let data = &bytes[data_start..data_end];
        match kind {
            b"IHDR" => {
                if len != 13 {
                    return Err("invalid IHDR");
                }
                width = be_u32(&data[0..4]);
                height = be_u32(&data[4..8]);
                let bit_depth = data[8];
                color_type = data[9];
                let compression = data[10];
                let filter_method = data[11];
                let interlace = data[12];
                if width == 0 || height == 0 || width > MAX_IMAGE_DIM || height > MAX_IMAGE_DIM {
                    return Err("unsupported PNG dimensions");
                }
                if compression != 0 || filter_method != 0 || interlace != 0 {
                    return Err("unsupported PNG compression/interlacing");
                }
                if bit_depth != 8 {
                    return Err("only 8-bit PNG supported");
                }
                saw_ihdr = true;
            }
            b"PLTE" => {
                if len % 3 != 0 {
                    return Err("invalid PLTE");
                }
                palette = data.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            }
            b"tRNS" => {
                trns = data.to_vec();
            }
            b"IDAT" => {
                idat.extend_from_slice(data);
            }
            b"IEND" => break,
            _ => {}
        }
        pos = data_end + 4;
    }

    if !saw_ihdr || idat.is_empty() {
        return Err("missing IHDR or IDAT");
    }

    let bpp: usize = match color_type {
        0 => 1, // Grayscale
        2 => 3, // RGB
        3 => 1, // Indexed
        4 => 2, // Grayscale + Alpha
        6 => 4, // RGBA
        _ => return Err("unsupported PNG color type"),
    };

    let row_bytes = (width as usize).checked_mul(bpp).ok_or("row overflow")?;
    let expected = (row_bytes + 1).checked_mul(height as usize).ok_or("image overflow")?;
    let inflated = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&idat, expected.max(1024 * 1024) + 1024)
        .map_err(|_| "PNG zlib inflate failed")?;
    if inflated.len() < expected {
        return Err("truncated inflated PNG data");
    }

    let mut prev = vec![0u8; row_bytes];
    let mut curr = vec![0u8; row_bytes];
    let mut bgra = vec![0u8; (width as usize) * (height as usize) * 4];
    let (bg_r, bg_g, bg_b) = bg_rgb;

    for y in 0..height as usize {
        let src_row = &inflated[y * (row_bytes + 1)..(y + 1) * (row_bytes + 1)];
        let filter = src_row[0];
        let raw = &src_row[1..];
        for i in 0..row_bytes {
            let a = if i >= bpp { curr[i - bpp] } else { 0 };
            let b = prev[i];
            let c = if i >= bpp { prev[i - bpp] } else { 0 };
            curr[i] = match filter {
                0 => raw[i],
                1 => raw[i].wrapping_add(a),
                2 => raw[i].wrapping_add(b),
                3 => raw[i].wrapping_add(((a as u16 + b as u16) >> 1) as u8),
                4 => raw[i].wrapping_add(paeth(a, b, c)),
                _ => return Err("invalid PNG filter"),
            };
        }

        // Bottom-up row index for Windows DIB
        let dst_y = (height as usize - 1) - y;
        let dst_row = &mut bgra[dst_y * (width as usize) * 4..(dst_y + 1) * (width as usize) * 4];
        for x in 0..width as usize {
            let (r, g, b, a) = match color_type {
                0 => {
                    let v = curr[x];
                    (v, v, v, 255u8)
                }
                2 => (curr[x * 3], curr[x * 3 + 1], curr[x * 3 + 2], 255u8),
                3 => {
                    let idx = curr[x] as usize;
                    let [r, g, b] = palette.get(idx).copied().unwrap_or([0, 0, 0]);
                    let a = trns.get(idx).copied().unwrap_or(255);
                    (r, g, b, a)
                }
                4 => {
                    let v = curr[x * 2];
                    (v, v, v, curr[x * 2 + 1])
                }
                6 => (curr[x * 4], curr[x * 4 + 1], curr[x * 4 + 2], curr[x * 4 + 3]),
                _ => (0, 0, 0, 255),
            };
            let (or, og, ob) = if a == 255 {
                (r, g, b)
            } else if a == 0 {
                (bg_r, bg_g, bg_b)
            } else {
                let alpha = a as u32;
                let inv = 255 - alpha;
                (
                    ((r as u32 * alpha + bg_r as u32 * inv) / 255) as u8,
                    ((g as u32 * alpha + bg_g as u32 * inv) / 255) as u8,
                    ((b as u32 * alpha + bg_b as u32 * inv) / 255) as u8,
                )
            };
            dst_row[x * 4] = ob;
            dst_row[x * 4 + 1] = og;
            dst_row[x * 4 + 2] = or;
            dst_row[x * 4 + 3] = 255;
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    Ok(DibImage { width, height, bgra_bottom_up: bgra })
}

pub fn load_png_file(path: &Path, bg_rgb: (u8, u8, u8)) -> Result<DibImage, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_PNG_FILE_BYTES {
        return Err("image file exceeds 16 MiB".into());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    decode_png(&bytes, bg_rgb).map_err(|e| e.to_owned())
}

// ---------------------------------------------------------------------------
// Minimal ISO/IEC 18004 QR Code encoder (Byte mode, ECC Level L, Versions 1..6)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct QrCode {
    pub size: usize,
    /// Row-major booleans: `true` = dark module, `false` = light module.
    pub modules: Vec<bool>,
}

impl QrCode {
    pub fn at(&self, x: usize, y: usize) -> bool {
        self.modules[y * self.size + x]
    }
}

struct VersionSpec {
    version: usize,
    total_data_bytes: usize,
    ecc_bytes_per_block: usize,
    num_blocks: usize,
    align_pos: &'static [usize],
}

// Level L capacities (ISO/IEC 18004 Table 9): Versions 1..6 (max 134 payload bytes).
const SPECS: &[VersionSpec] = &[
    VersionSpec { version: 1, total_data_bytes: 19,  ecc_bytes_per_block: 7,  num_blocks: 1, align_pos: &[] },
    VersionSpec { version: 2, total_data_bytes: 34,  ecc_bytes_per_block: 10, num_blocks: 1, align_pos: &[6, 18] },
    VersionSpec { version: 3, total_data_bytes: 55,  ecc_bytes_per_block: 15, num_blocks: 1, align_pos: &[6, 22] },
    VersionSpec { version: 4, total_data_bytes: 80,  ecc_bytes_per_block: 20, num_blocks: 1, align_pos: &[6, 26] },
    VersionSpec { version: 5, total_data_bytes: 108, ecc_bytes_per_block: 26, num_blocks: 1, align_pos: &[6, 30] },
    VersionSpec { version: 6, total_data_bytes: 136, ecc_bytes_per_block: 18, num_blocks: 2, align_pos: &[6, 34] },
];

fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut r = 0u8;
    for _ in 0..8 {
        if (b & 1) != 0 {
            r ^= a;
        }
        let hi = (a & 0x80) != 0;
        a <<= 1;
        if hi {
            a ^= 0x1d; // x^8 + x^4 + x^3 + x^2 + 1
        }
        b >>= 1;
    }
    r
}

fn rs_generator(degree: usize) -> Vec<u8> {
    let mut poly = vec![0u8; degree];
    poly[degree - 1] = 1;
    let mut root = 1u8;
    for _ in 0..degree {
        for j in 0..degree {
            poly[j] = gf_mul(poly[j], root);
            if j + 1 < degree {
                poly[j] ^= poly[j + 1];
            }
        }
        root = gf_mul(root, 2);
    }
    poly
}

fn rs_remainder(data: &[u8], degree: usize) -> Vec<u8> {
    let gen = rs_generator(degree);
    let mut rem = vec![0u8; degree];
    for &b in data {
        let factor = b ^ rem[0];
        rem.rotate_left(1);
        rem[degree - 1] = 0;
        for (r, &g) in rem.iter_mut().zip(gen.iter()) {
            *r ^= gf_mul(g, factor);
        }
    }
    rem
}

fn push_bits(out: &mut Vec<bool>, value: u32, count: usize) {
    for i in (0..count).rev() {
        out.push(((value >> i) & 1) != 0);
    }
}

fn draw_finder(modules: &mut [bool], is_func: &mut [bool], size: usize, cx: isize, cy: isize) {
    for dy in -4..=4 {
        for dx in -4..=4 {
            let x = cx + dx;
            let y = cy + dy;
            if x >= 0 && y >= 0 && (x as usize) < size && (y as usize) < size {
                let dist = dx.abs().max(dy.abs());
                let dark = dist != 2 && dist != 4;
                let idx = (y as usize) * size + (x as usize);
                modules[idx] = dark;
                is_func[idx] = true;
            }
        }
    }
}

fn draw_alignment(modules: &mut [bool], is_func: &mut [bool], size: usize, cx: usize, cy: usize) {
    for dy in -2isize..=2 {
        for dx in -2isize..=2 {
            let x = (cx as isize + dx) as usize;
            let y = (cy as isize + dy) as usize;
            let dist = dx.abs().max(dy.abs());
            let idx = y * size + x;
            modules[idx] = dist != 1;
            is_func[idx] = true;
        }
    }
}

fn format_bits_level_l(mask: u32) -> u32 {
    // Error correction level L = 0b01
    let data = (0b01 << 3) | (mask & 7);
    let mut rem = data;
    for _ in 0..10 {
        rem = (rem << 1) ^ (((rem >> 9) & 1) * 0x537);
    }
    ((data << 10) | rem) ^ 0x5412
}

fn write_format(modules: &mut [bool], is_func: &mut [bool], size: usize, mask: u32) {
    let bits = format_bits_level_l(mask);
    // Around top-left finder
    for i in 0..6 {
        let bit = ((bits >> i) & 1) != 0;
        modules[i * size + 8] = bit;
        is_func[i * size + 8] = true;
    }
    modules[7 * size + 8] = ((bits >> 6) & 1) != 0;
    is_func[7 * size + 8] = true;
    modules[8 * size + 8] = ((bits >> 7) & 1) != 0;
    is_func[8 * size + 8] = true;
    modules[8 * size + 7] = ((bits >> 8) & 1) != 0;
    is_func[8 * size + 7] = true;
    for i in 9..15 {
        let bit = ((bits >> i) & 1) != 0;
        modules[8 * size + (14 - i)] = bit;
        is_func[8 * size + (14 - i)] = true;
    }
    // Right of top-right + bottom of bottom-left
    for i in 0..8 {
        let bit = ((bits >> i) & 1) != 0;
        modules[8 * size + (size - 1 - i)] = bit;
        is_func[8 * size + (size - 1 - i)] = true;
    }
    for i in 8..15 {
        let bit = ((bits >> i) & 1) != 0;
        modules[(size - 15 + i) * size + 8] = bit;
        is_func[(size - 15 + i) * size + 8] = true;
    }
    // Always-dark module
    modules[(size - 8) * size + 8] = true;
    is_func[(size - 8) * size + 8] = true;
}

fn mask_condition(mask: u32, x: usize, y: usize) -> bool {
    match mask {
        0 => (x + y) % 2 == 0,
        1 => y % 2 == 0,
        2 => x % 3 == 0,
        3 => (x + y) % 3 == 0,
        4 => ((y / 2) + (x / 3)) % 2 == 0,
        5 => ((x * y) % 2) + ((x * y) % 3) == 0,
        6 => (((x * y) % 2) + ((x * y) % 3)) % 2 == 0,
        _ => (((x + y) % 2) + ((x * y) % 3)) % 2 == 0,
    }
}

fn penalty_score(modules: &[bool], size: usize) -> usize {
    let mut score = 0usize;
    // Rule 1: horizontal and vertical runs >= 5
    for y in 0..size {
        let mut run = 1;
        for x in 1..size {
            if modules[y * size + x] == modules[y * size + x - 1] {
                run += 1;
            } else {
                if run >= 5 {
                    score += 3 + (run - 5);
                }
                run = 1;
            }
        }
        if run >= 5 {
            score += 3 + (run - 5);
        }
    }
    for x in 0..size {
        let mut run = 1;
        for y in 1..size {
            if modules[y * size + x] == modules[(y - 1) * size + x] {
                run += 1;
            } else {
                if run >= 5 {
                    score += 3 + (run - 5);
                }
                run = 1;
            }
        }
        if run >= 5 {
            score += 3 + (run - 5);
        }
    }
    // Rule 2: 2x2 uniform blocks
    for y in 0..size - 1 {
        for x in 0..size - 1 {
            let c = modules[y * size + x];
            if modules[y * size + x + 1] == c
                && modules[(y + 1) * size + x] == c
                && modules[(y + 1) * size + x + 1] == c
            {
                score += 3;
            }
        }
    }
    score
}

/// Encode UTF-8 text (up to 134 bytes) as a standard QR Code (ECC Level L).
pub fn encode_qr(text: &str) -> Option<QrCode> {
    let payload = text.as_bytes();
    // Byte mode header: 4-bit mode (0100) + 8-bit length (for versions 1..9)
    let needed_bits = 4 + 8 + payload.len() * 8;
    let spec = SPECS.iter().find(|s| s.total_data_bytes * 8 >= needed_bits)?;

    let mut bits = Vec::with_capacity(spec.total_data_bytes * 8);
    push_bits(&mut bits, 0b0100, 4);
    push_bits(&mut bits, payload.len() as u32, 8);
    for &b in payload {
        push_bits(&mut bits, b as u32, 8);
    }
    let capacity_bits = spec.total_data_bytes * 8;
    let terminator = (capacity_bits - bits.len()).min(4);
    push_bits(&mut bits, 0, terminator);
    while bits.len() % 8 != 0 {
        bits.push(false);
    }
    let mut data = Vec::with_capacity(spec.total_data_bytes);
    for chunk in bits.chunks_exact(8) {
        let mut byte = 0u8;
        for &b in chunk {
            byte = (byte << 1) | (b as u8);
        }
        data.push(byte);
    }
    let mut pad_toggle = false;
    while data.len() < spec.total_data_bytes {
        data.push(if pad_toggle { 0x11 } else { 0xEC });
        pad_toggle = !pad_toggle;
    }

    // Split into blocks and compute Reed-Solomon ECC
    let block_data_len = spec.total_data_bytes / spec.num_blocks;
    let mut data_blocks = Vec::with_capacity(spec.num_blocks);
    let mut ecc_blocks = Vec::with_capacity(spec.num_blocks);
    for i in 0..spec.num_blocks {
        let slice = &data[i * block_data_len..(i + 1) * block_data_len];
        data_blocks.push(slice.to_vec());
        ecc_blocks.push(rs_remainder(slice, spec.ecc_bytes_per_block));
    }

    // Interleave data codewords, then ECC codewords
    let mut codewords = Vec::with_capacity(spec.total_data_bytes + spec.num_blocks * spec.ecc_bytes_per_block);
    for col in 0..block_data_len {
        for blk in &data_blocks {
            codewords.push(blk[col]);
        }
    }
    for col in 0..spec.ecc_bytes_per_block {
        for blk in &ecc_blocks {
            codewords.push(blk[col]);
        }
    }

    let size = spec.version * 4 + 17;
    let mut base_modules = vec![false; size * size];
    let mut is_func = vec![false; size * size];

    // Timing patterns
    for i in 0..size {
        let dark = i % 2 == 0;
        base_modules[6 * size + i] = dark;
        is_func[6 * size + i] = true;
        base_modules[i * size + 6] = dark;
        is_func[i * size + 6] = true;
    }

    // Finder patterns
    draw_finder(&mut base_modules, &mut is_func, size, 3, 3);
    draw_finder(&mut base_modules, &mut is_func, size, (size - 4) as isize, 3);
    draw_finder(&mut base_modules, &mut is_func, size, 3, (size - 4) as isize);

    // Alignment patterns
    for &ay in spec.align_pos {
        for &ax in spec.align_pos {
            let overlaps_finder = (ax == 6 && ay == 6)
                || (ax == 6 && ay + 4 >= size - 4)
                || (ay == 6 && ax + 4 >= size - 4);
            if !overlaps_finder {
                draw_alignment(&mut base_modules, &mut is_func, size, ax, ay);
            }
        }
    }

    // Reserve format area (dummy mask 0 for now)
    write_format(&mut base_modules, &mut is_func, size, 0);

    // Place data bits in zigzag columns right-to-left
    let mut bit_idx = 0usize;
    let total_bits = codewords.len() * 8;
    let mut col = size as isize - 1;
    let mut upward = true;
    while col >= 1 {
        if col == 6 {
            col -= 1;
        }
        for step in 0..size {
            let y = if upward { size - 1 - step } else { step };
            for dx in 0..2isize {
                let x = (col - dx) as usize;
                let idx = y * size + x;
                if !is_func[idx] {
                    let bit = if bit_idx < total_bits {
                        ((codewords[bit_idx >> 3] >> (7 - (bit_idx & 7))) & 1) != 0
                    } else {
                        false
                    };
                    base_modules[idx] = bit;
                    bit_idx += 1;
                }
            }
        }
        col -= 2;
        upward = !upward;
    }

    // Choose lowest-penalty mask (0..8)
    let mut best_modules = Vec::new();
    let mut best_score = usize::MAX;
    for mask in 0..8u32 {
        let mut candidate = base_modules.clone();
        for y in 0..size {
            for x in 0..size {
                let idx = y * size + x;
                if !is_func[idx] && mask_condition(mask, x, y) {
                    candidate[idx] = !candidate[idx];
                }
            }
        }
        write_format(&mut candidate, &mut is_func.clone(), size, mask);
        let score = penalty_score(&candidate, size);
        if score < best_score {
            best_score = score;
            best_modules = candidate;
        }
    }

    Some(QrCode { size, modules: best_modules })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if (crc & 1) != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    fn make_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc_in = Vec::with_capacity(4 + data.len());
        crc_in.extend_from_slice(kind);
        crc_in.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
        out
    }

    #[test]
    fn decodes_rgba_png_and_blends_alpha() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        // 2x1 RGBA
        let ihdr = [0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0, 0, 0];
        png.extend_from_slice(&make_chunk(b"IHDR", &ihdr));
        // Filter=0, pixel 0 = solid red (255,0,0,255), pixel 1 = transparent (0,255,0,0)
        let raw = [0u8, 255, 0, 0, 255, 0, 255, 0, 0];
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        png.extend_from_slice(&make_chunk(b"IDAT", &z));
        png.extend_from_slice(&make_chunk(b"IEND", &[]));

        let img = decode_png(&png, (10, 20, 30)).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        // BGRA pixel 0 = (0, 0, 255, 255)
        assert_eq!(&img.bgra_bottom_up[0..4], &[0, 0, 255, 255]);
        // BGRA pixel 1 = background (30, 20, 10, 255)
        assert_eq!(&img.bgra_bottom_up[4..8], &[30, 20, 10, 255]);
    }

    #[test]
    fn encodes_qr_code_with_valid_finder_patterns() {
        let qr = encode_qr("http://192.168.1.50:8360/?pin=123456").unwrap();
        assert_eq!(qr.size, 29); // Version 3 (36 bytes > 34 bytes for V2)
        // Top-left finder center (3,3) is dark, ring at (1,1) is light, border (0,0) is dark
        assert!(qr.at(0, 0));
        assert!(!qr.at(1, 1));
        assert!(qr.at(3, 3));
        // Always-dark module at (8, size - 8)
        assert!(qr.at(8, qr.size - 8));
    }
}
