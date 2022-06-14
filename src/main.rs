use lodepng::*;
use serde::Deserialize;
use structopt::StructOpt;

use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(StructOpt, Debug)]
struct Options {
    /// Find assets in this directory, recursively.
    assets_dir: PathBuf,

    /// Write compiled assets to this file.
    out_file: PathBuf,
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

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let opts = Options::from_args();

    let mut builder = SpritesBuilder::default();

    for entry in walkdir::WalkDir::new(&opts.assets_dir) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            continue;
        }
        if entry.path().extension() != Some(OsStr::new("yml")) {
            continue;
        }

        let desc: SpritesDesc = serde_yaml::from_reader(File::open(entry.path())?)?;
        let bmp = decode24_file(entry.path().with_extension("png"))?;
        builder.add(desc, bmp);
    }

    let compiled = builder.compile()?;
    compiled.write_to(&opts.out_file)?;

    Ok(())
}

#[derive(Default)]
struct SpritesBuilder {
    sprites: HashMap<String, Bitmap<RGB<u8>>>,
}

impl SpritesBuilder {
    fn add(&mut self, sprites: SpritesDesc, bmp: Bitmap<RGB<u8>>) {
        for (id, desc) in sprites.sprites {
            self.sprites.insert(id, extract_rect(&bmp, desc.rect));
        }
    }

    fn compile(self) -> Result<CompiledSprites> {
        let mut compiled = CompiledSprites {
            next_palette_index: Some(0),
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
    fn palette_index(&mut self, color: RGB<u8>) -> Result<u8> {
        use std::collections::hash_map::Entry;

        match (self.palette.entry(color), self.next_palette_index) {
            (Entry::Occupied(o), _) => Ok(*o.get()),
            (Entry::Vacant(e), Some(i)) => {
                self.next_palette_index = i.checked_add(1);
                e.insert(i);
                Ok(i)
            }
            (Entry::Vacant(_), None) => Err("Found more than 256 colors".into()),
        }
    }

    fn write_to(&self, path: impl AsRef<Path>) -> Result<()> {
        let mut file = File::create(path)?;
        writeln!(file, "use ::gba::mmio_types::Color;\n")?;

        let mut unwritten_colors = self
            .palette
            .iter()
            .map(|(&c, &i)| (i, c))
            .collect::<HashMap<_, _>>();

        writeln!(file, "pub const PALETTE: &'static [Color] = &[")?;
        for i in 0..=255 {
            if unwritten_colors.is_empty() {
                break;
            }
            let c = unwritten_colors.remove(&i).unwrap_or(RGB::default());
            writeln!(
                file,
                "    Color::from_rgb({}, {}, {}),",
                c.r >> 3,
                c.g >> 3,
                c.b >> 3,
            )?;
        }
        writeln!(file, "];\n")?;

        for (id, tiles) in &self.sprites {
            writeln!(
                file,
                "pub const {}: &'static [[u8; 64]] = &[",
                id.to_uppercase()
            )?;

            for tile in tiles {
                writeln!(file, "    {:?},", tile)?;
            }

            writeln!(file, "];")?;
        }

        Ok(())
    }
}
