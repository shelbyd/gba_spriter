// #![cfg_attr(not(feature = "std"), no_std)]

use proc_macro::TokenStream;

#[proc_macro]
pub fn compile(input: TokenStream) -> TokenStream {
    let in_dir = syn::parse_macro_input!(input as syn::LitStr);

    match compile_internal(&in_dir.value()) {
        Ok(s) => s.parse().unwrap(),
        Err(e) => {
            return proc_macro::TokenStream::from(
                syn::Error::new(in_dir.span(), e).to_compile_error(),
            );
        }
    }
}

use lodepng::*;
use serde::Deserialize;

use std::{collections::HashMap, ffi::OsStr, fmt::Write, fs::File, path::Path};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn compile_internal(dir: &str) -> Result<String> {
    let mut out = String::new();
    let mut builder = SpritesBuilder::default();

    for entry in walkdir::WalkDir::new(dir) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            continue;
        }
        if entry.path().extension() != Some(OsStr::new("yml")) {
            continue;
        }

        let desc_path = use_path(&mut out, entry.path())?;
        let png_path = use_path(&mut out, entry.path().with_extension("png"))?;

        let desc: SpritesDesc = serde_yaml::from_reader(File::open(desc_path)?)?;
        let bmp = decode32_file(png_path)?;
        builder.add(desc, bmp);
    }

    let compiled = builder.compile()?;
    compiled.write_to(&mut out)?;
    Ok(out)
}

fn use_path<P: AsRef<Path>>(mut out: impl Write, p: P) -> std::result::Result<P, std::fmt::Error> {
    // TODO(shelbyd): Import relative to actual root.
    write!(
        &mut out,
        "const _: &[u8] = include_bytes!(\"../{}\");",
        p.as_ref().display(),
    )?;

    Ok(p)
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct SpritesDesc {
    sprites: HashMap<String, Sprite>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct Sprite {
    rect: (usize, usize, usize, usize),
}

#[derive(Default)]
struct SpritesBuilder {
    sprites: HashMap<String, Bitmap<RGBA>>,
}

impl SpritesBuilder {
    fn add(&mut self, sprites: SpritesDesc, bmp: Bitmap<RGBA>) {
        for (id, desc) in sprites.sprites {
            self.sprites.insert(id, extract_rect(&bmp, desc.rect));
        }
    }

    fn compile(self) -> Result<CompiledSprites> {
        let mut compiled = CompiledSprites {
            next_palette_index: Some(1),
            palette: HashMap::new(),
            sprites: HashMap::new(),
        };

        for (id, bmp) in self.sprites {
            let x_tiles = bmp.width / 8;
            let y_tiles = bmp.height / 8;

            let mut tiles = Vec::new();
            for y_tile in 0..y_tiles {
                for x_tile in 0..x_tiles {
                    let tile = extract_rect(&bmp, (x_tile * 8, y_tile * 8, 8, 8))
                        .buffer
                        .into_iter()
                        .map(|c| compiled.palette_index(c))
                        .collect::<Result<_>>()?;
                    tiles.push(tile);
                }
            }
            compiled.sprites.insert(id, tiles);
        }

        Ok(compiled)
    }
}

fn extract_rect<P: Copy>(
    input: &Bitmap<P>,
    (base_x, base_y, w, h): (usize, usize, usize, usize),
) -> Bitmap<P> {
    let mut result = Bitmap {
        buffer: Vec::new(),
        width: w,
        height: h,
    };
    let buf = &mut result.buffer;

    for y in base_y..(base_y + h) {
        for x in base_x..(base_x + w) {
            let index = y * input.width + x;
            buf.push(input.buffer[index]);
        }
    }

    result
}

#[derive(Debug)]
struct CompiledSprites {
    next_palette_index: Option<u8>,
    palette: HashMap<RGB<u8>, u8>,
    sprites: HashMap<String, Vec<Vec<u8>>>,
}

impl CompiledSprites {
    fn palette_index(&mut self, color: RGBA) -> Result<u8> {
        use std::collections::hash_map::Entry;

        if color.a == 0 {
            return Ok(0);
        }
        assert_eq!(color.a, 255);

        match (self.palette.entry(color.rgb()), self.next_palette_index) {
            (Entry::Occupied(o), _) => Ok(*o.get()),
            (Entry::Vacant(e), Some(i)) => {
                self.next_palette_index = i.checked_add(1);
                e.insert(i);
                Ok(i)
            }
            (Entry::Vacant(_), None) => Err("Too many colors to fit into single palette".into()),
        }
    }

    fn write_to(&self, mut out: impl std::fmt::Write) -> Result<()> {
        writeln!(out, "use ::gba::mmio_types::Color;\n")?;

        let mut unwritten_colors = self
            .palette
            .iter()
            .map(|(&c, &i)| (i, c))
            .collect::<HashMap<_, _>>();

        writeln!(out, "pub const PALETTE: &'static [Color] = &[")?;
        for i in 0..=255 {
            if unwritten_colors.is_empty() {
                break;
            }
            let c = unwritten_colors.remove(&i).unwrap_or(RGB::default());
            writeln!(
                out,
                "    Color::from_rgb({}, {}, {}),",
                c.r >> 3,
                c.g >> 3,
                c.b >> 3,
            )?;
        }
        writeln!(out, "];\n")?;

        for (id, tiles) in &self.sprites {
            writeln!(
                out,
                "pub const {}: &'static [[u8; 64]] = &[",
                id.to_uppercase()
            )?;

            for tile in tiles {
                writeln!(out, "    {:?},", tile)?;
            }

            writeln!(out, "];")?;
        }

        Ok(())
    }
}
