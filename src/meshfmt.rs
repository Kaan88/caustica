//! PLY and STL readers (closing the medium term, core track 2 importer gaps). Both give
//! indexed triangles for `mesh::triangles`: PLY (ASCII and binary, either byte order) with
//! optional normals, texture coordinates and vertex colours (averaged into the material
//! colour, as vertex colours don't exist here); STL (ASCII and binary) welded by position,
//! with normals smoothed across edges under 30° so scans come out smooth and CAD edges stay
//! sharp.
use anyhow::{Context, Result, bail, ensure};
use glam::Vec3;
use std::collections::HashMap;

/// Indexed triangles as read.
#[derive(Default)]
pub struct Indexed {
    pub positions: Vec<Vec3>,
    pub normals: Option<Vec<Vec3>>,
    pub uvs: Option<Vec<[f32; 2]>>,
    pub indices: Vec<u32>,
    /// The mean vertex colour (linear), when the file has colours.
    pub color: Option<[f32; 3]>,
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Ascii,
    Little,
    Big,
}
#[derive(Clone, Copy)]
enum Scalar {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}
impl Scalar {
    fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "char" | "int8" => Self::I8,
            "uchar" | "uint8" => Self::U8,
            "short" | "int16" => Self::I16,
            "ushort" | "uint16" => Self::U16,
            "int" | "int32" => Self::I32,
            "uint" | "uint32" => Self::U32,
            "float" | "float32" => Self::F32,
            "double" | "float64" => Self::F64,
            _ => bail!("PLY: unknown type {name}"),
        })
    }
    fn size(self) -> usize {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::F64 => 8,
        }
    }
    /// Integer colour channels are 0–255 (or 0–65535); floats are 0–1.
    fn max(self) -> f64 {
        match self {
            Self::U8 | Self::I8 => 255.,
            Self::U16 | Self::I16 => 65535.,
            _ => 1.,
        }
    }
}
struct Property {
    name: String,
    kind: Scalar,
    /// A list property: the type of its count.
    list: Option<Scalar>,
}
struct Element {
    name: String,
    count: usize,
    properties: Vec<Property>,
}

/// Reads values from the body, ASCII tokens or binary in either byte order.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
    format: Format,
}
impl Reader<'_> {
    fn token(&mut self) -> Result<&str> {
        while self.at < self.data.len() && self.data[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
        let start = self.at;
        while self.at < self.data.len() && !self.data[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
        ensure!(self.at > start, "PLY: the file ends early");
        std::str::from_utf8(&self.data[start..self.at]).context("PLY: bad text")
    }
    fn read(&mut self, kind: Scalar) -> Result<f64> {
        if self.format == Format::Ascii {
            return self.token()?.parse::<f64>().context("PLY: bad number");
        }
        let n = kind.size();
        ensure!(self.at + n <= self.data.len(), "PLY: the file ends early");
        let mut b = [0u8; 8];
        b[..n].copy_from_slice(&self.data[self.at..self.at + n]);
        if self.format == Format::Big {
            b[..n].reverse();
        }
        self.at += n;
        Ok(match kind {
            Scalar::I8 => b[0] as i8 as f64,
            Scalar::U8 => b[0] as f64,
            Scalar::I16 => i16::from_le_bytes([b[0], b[1]]) as f64,
            Scalar::U16 => u16::from_le_bytes([b[0], b[1]]) as f64,
            Scalar::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Scalar::U32 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Scalar::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Scalar::F64 => f64::from_le_bytes(b),
        })
    }
}

pub fn read_ply(data: &[u8]) -> Result<Indexed> {
    let end = data
        .windows(10)
        .position(|w| w == b"end_header")
        .context("PLY: no end_header")?;
    let header = std::str::from_utf8(&data[..end]).context("PLY: bad header")?;
    let mut body = end + 10;
    // The header ends with a newline (\n or \r\n).
    while body < data.len() && (data[body] == b'\r' || data[body] == b'\n') {
        body += 1;
        if data[body - 1] == b'\n' {
            break;
        }
    }
    let mut lines = header.lines();
    ensure!(lines.next().map(str::trim) == Some("ply"), "Not a PLY file");
    let mut format = Format::Ascii;
    let mut elements: Vec<Element> = vec![];
    for line in lines {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["format", f, ..] => {
                format = match *f {
                    "ascii" => Format::Ascii,
                    "binary_little_endian" => Format::Little,
                    "binary_big_endian" => Format::Big,
                    _ => bail!("PLY: unknown format {f}"),
                }
            }
            ["element", name, count] => elements.push(Element {
                name: name.to_string(),
                count: count.parse().context("PLY: bad element count")?,
                properties: vec![],
            }),
            ["property", "list", count, kind, name] => elements
                .last_mut()
                .context("PLY: property before element")?
                .properties
                .push(Property {
                    name: name.to_string(),
                    kind: Scalar::parse(kind)?,
                    list: Some(Scalar::parse(count)?),
                }),
            ["property", kind, name] => elements
                .last_mut()
                .context("PLY: property before element")?
                .properties
                .push(Property {
                    name: name.to_string(),
                    kind: Scalar::parse(kind)?,
                    list: None,
                }),
            _ => {}
        }
    }
    let mut r = Reader {
        data,
        at: body,
        format,
    };
    let mut out = Indexed::default();
    let (mut normals, mut uvs, mut colors) = (vec![], vec![], vec![]);
    for e in &elements {
        let find = |names: &[&str]| e.properties.iter().position(|p| names.contains(&&*p.name));
        let (x, y, z) = (find(&["x"]), find(&["y"]), find(&["z"]));
        let (nx, ny, nz) = (find(&["nx"]), find(&["ny"]), find(&["nz"]));
        let (u, v) = (
            find(&["u", "s", "texture_u", "texture_s"]),
            find(&["v", "t", "texture_v", "texture_t"]),
        );
        let (cr, cg, cb) = (
            find(&["red", "r", "diffuse_red"]),
            find(&["green", "g", "diffuse_green"]),
            find(&["blue", "b", "diffuse_blue"]),
        );
        let faces = e.name == "face";
        let face_list = find(&["vertex_indices", "vertex_index"]);
        ensure!(
            e.count <= crate::mesh::MAX_TRIANGLES * 3,
            "PLY: too many {}",
            e.name
        );
        let mut values = vec![0f64; e.properties.len()];
        let mut list = vec![];
        for _ in 0..e.count {
            for (k, p) in e.properties.iter().enumerate() {
                match p.list {
                    Some(count) => {
                        let n = r.read(count)? as usize;
                        ensure!(n <= 1 << 16, "PLY: a list of {n} entries");
                        if faces && Some(k) == face_list {
                            list.clear();
                            for _ in 0..n {
                                list.push(r.read(p.kind)? as u32);
                            }
                        } else {
                            for _ in 0..n {
                                r.read(p.kind)?;
                            }
                        }
                    }
                    None => values[k] = r.read(p.kind)?,
                }
            }
            if e.name == "vertex" {
                let g = |i: Option<usize>| i.map_or(0., |i| values[i] as f32);
                out.positions.push(Vec3::new(g(x), g(y), g(z)));
                if nx.is_some() && ny.is_some() && nz.is_some() {
                    normals.push(Vec3::new(g(nx), g(ny), g(nz)));
                }
                if let (Some(_), Some(_)) = (u, v) {
                    // PLY's v runs up the image, D3D's down (as OBJ).
                    uvs.push([g(u), 1. - g(v)]);
                }
                if let (Some(a), Some(b), Some(c)) = (cr, cg, cb) {
                    let m = e.properties[a].kind.max();
                    colors.push([a, b, c].map(|i| (values[i] / m) as f32));
                }
            } else if faces && list.len() >= 3 {
                // A polygon as a fan.
                for k in 1..list.len() - 1 {
                    out.indices.extend([list[0], list[k], list[k + 1]]);
                }
            }
        }
    }
    let n = out.positions.len();
    ensure!(
        out.indices.iter().all(|&i| (i as usize) < n),
        "PLY: a face points past the vertices"
    );
    out.normals = (normals.len() == n && n > 0).then_some(normals);
    out.uvs = (uvs.len() == n && n > 0).then_some(uvs);
    if colors.len() == n && n > 0 {
        let mut sum = [0f64; 3];
        for c in &colors {
            for k in 0..3 {
                sum[k] += srgb_to_linear(c[k].clamp(0., 1.)) as f64;
            }
        }
        out.color = Some(sum.map(|s| (s / n as f64) as f32));
    }
    Ok(out)
}
fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

pub fn read_stl(data: &[u8]) -> Result<Indexed> {
    let mut corners: Vec<Vec3> = vec![];
    // Binary: the count matches the size, or the file doesn't start as text and holds at
    // least that many triangles (some writers pad the end).
    let binary = data.len() >= 84 && {
        let n = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;
        84 + n * 50 == data.len() || (!data.starts_with(b"solid") && 84 + n * 50 <= data.len())
    };
    if binary {
        let n = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;
        ensure!(n <= crate::mesh::MAX_TRIANGLES, "STL: too many triangles");
        for t in data[84..84 + n * 50].as_chunks::<50>().0 {
            for v in 0..3 {
                let f = |k: usize| {
                    let o = 12 + v * 12 + k * 4;
                    f32::from_le_bytes([t[o], t[o + 1], t[o + 2], t[o + 3]])
                };
                corners.push(Vec3::new(f(0), f(1), f(2)));
            }
        }
    } else {
        let text = std::str::from_utf8(data).context("STL: neither binary nor text")?;
        ensure!(text.trim_start().starts_with("solid"), "Not an STL file");
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            if let ["vertex", x, y, z] = w.as_slice() {
                let p = |s: &str| s.parse::<f32>().context("STL: bad number");
                corners.push(Vec3::new(p(x)?, p(y)?, p(z)?));
            }
        }
        ensure!(
            corners.len().is_multiple_of(3),
            "STL: a facet without three vertices"
        );
    }
    Ok(smooth(corners, 30f32.to_radians().cos()))
}
/// Welds a triangle soup by position and gives each corner the area-weighted normal of the
/// faces around its vertex that turn less than the threshold from its own face.
fn smooth(corners: Vec<Vec3>, cos_limit: f32) -> Indexed {
    let faces: Vec<Vec3> = corners
        .as_chunks::<3>()
        .0
        .iter()
        .map(|[a, b, c]| (*b - *a).cross(*c - *a))
        .collect();
    let mut ids: HashMap<[u32; 3], u32> = HashMap::new();
    let welded: Vec<u32> = corners
        .iter()
        .map(|p| {
            let n = ids.len() as u32;
            // + 0.0 turns -0.0 into 0.0, so both weld.
            *ids.entry(p.to_array().map(|v| (v + 0.).to_bits()))
                .or_insert(n)
        })
        .collect();
    let mut around: Vec<Vec<u32>> = vec![vec![]; ids.len()];
    for (k, &w) in welded.iter().enumerate() {
        around[w as usize].push((k / 3) as u32);
    }
    let normals: Vec<Vec3> = welded
        .iter()
        .enumerate()
        .map(|(k, &w)| {
            let own = faces[k / 3].normalize_or_zero();
            around[w as usize]
                .iter()
                .map(|&f| faces[f as usize])
                .filter(|n| n.normalize_or_zero().dot(own) >= cos_limit)
                .sum::<Vec3>()
                .normalize_or(if own == Vec3::ZERO { Vec3::Y } else { own })
        })
        .collect();
    Indexed {
        indices: (0..corners.len() as u32).collect(),
        positions: corners,
        normals: Some(normals),
        uvs: None,
        color: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ply_ascii_and_binary_agree() {
        let ascii = b"ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0 255 0 0\n1 0 0 255 0 0\n1 1 0 255 0 0\n0 1 0 255 0 0\n4 0 1 2 3\n";
        let a = read_ply(ascii).unwrap();
        assert_eq!(a.positions.len(), 4);
        assert_eq!(a.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(a.color, Some([1., 0., 0.]));
        let mut bin = b"ply\r\nformat binary_big_endian 1.0\r\nelement vertex 3\r\nproperty double x\r\nproperty double y\r\nproperty double z\r\nelement face 1\r\nproperty list uchar uint vertex_index\r\nend_header\r\n".to_vec();
        for p in [[0., 0., 0.], [1., 0., 0.], [1f64, 1., 0.]] {
            for v in p {
                bin.extend(v.to_be_bytes());
            }
        }
        bin.push(3);
        for i in [0u32, 1, 2] {
            bin.extend(i.to_be_bytes());
        }
        let b = read_ply(&bin).unwrap();
        assert_eq!(b.positions[2], Vec3::new(1., 1., 0.));
        assert_eq!(b.indices, vec![0, 1, 2]);
    }
    #[test]
    fn stl_welds_and_keeps_hard_edges() {
        // Two faces of a cube meeting at 90°: the shared corners keep each face's normal.
        let text = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 1 1 0\nendloop\nendfacet\nfacet normal 0 -1 0\nouter loop\nvertex 0 0 0\nvertex 1 0 -1\nvertex 1 0 0\nendloop\nendfacet\nendsolid t\n";
        let s = read_stl(text.as_bytes()).unwrap();
        let n = s.normals.unwrap();
        assert!((n[0] - Vec3::Z).length() < 1e-5 && (n[3] + Vec3::Y).length() < 1e-5);
        // Binary: the same first facet.
        let mut bin = vec![0u8; 80];
        bin.extend(1u32.to_le_bytes());
        for v in [0f32, 0., 1., 0., 0., 0., 1., 0., 0., 1., 1., 0.] {
            bin.extend(v.to_le_bytes());
        }
        bin.extend([0, 0]);
        let b = read_stl(&bin).unwrap();
        assert_eq!(b.positions[2], Vec3::new(1., 1., 0.));
    }
}
