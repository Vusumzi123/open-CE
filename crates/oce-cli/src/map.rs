use std::collections::HashMap;
use std::error::Error;

use minifb::{Key, KeyRepeat, Window, WindowOptions};
use oce_core::Result;
use oce_plugin::{Plugin, Record, WalkEntry};

const TAMRIEL: u32 = 0x0000_003C;
const WIDTH: usize = 1280;
const HEIGHT: usize = 720;
const BACKGROUND: u32 = 0x0007_0C10;
const GRID: u32 = 0x0010_1A1E;

#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
    pub refs: u32,
}

pub fn build_cells(plugin: &Plugin) -> Result<Vec<Cell>> {
    let mut cells = Vec::new();
    let mut by_grid = HashMap::new();
    let mut world = None;
    let mut current = None;

    plugin.walk(|entry| {
        match entry {
            WalkEntry::Group { header, depth, .. } => {
                if depth == 0 {
                    world = None;
                    current = None;
                } else if matches!(header.group_type, 1..=5) {
                    current = None;
                }
            }
            WalkEntry::Record {
                header,
                body,
                offset,
                ..
            } => {
                if &header.signature == b"WRLD" {
                    world = Some(header.form_id.raw());
                    current = None;
                } else if &header.signature == b"CELL" {
                    current = None;
                    if world == Some(TAMRIEL) {
                        let record = Record {
                            header,
                            body,
                            offset,
                        };
                        let decoded = record.decode()?;
                        let subrecords = decoded.subrecords()?;
                        if let Some(xclc) = subrecords.iter().find(|sub| &sub.signature == b"XCLC")
                        {
                            if xclc.data.len() >= 8 {
                                let x = i32::from_le_bytes(xclc.data[0..4].try_into().unwrap());
                                let y = i32::from_le_bytes(xclc.data[4..8].try_into().unwrap());
                                if (-256..=256).contains(&x) && (-256..=256).contains(&y) {
                                    let index = *by_grid.entry((x, y)).or_insert_with(|| {
                                        cells.push(Cell { x, y, refs: 0 });
                                        cells.len() - 1
                                    });
                                    current = Some(index);
                                }
                            }
                        }
                    }
                } else if &header.signature == b"REFR" {
                    if let Some(index) = current {
                        cells[index].refs = cells[index].refs.saturating_add(1);
                    }
                }
            }
        }
        Ok(())
    })?;

    Ok(cells)
}

#[derive(Clone, Copy, Debug)]
struct Bounds {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

impl Bounds {
    fn new(cells: &[Cell]) -> Option<Self> {
        let first = cells.first()?;
        let mut bounds = Self {
            min_x: first.x,
            max_x: first.x,
            min_y: first.y,
            max_y: first.y,
        };
        for cell in &cells[1..] {
            bounds.min_x = bounds.min_x.min(cell.x);
            bounds.max_x = bounds.max_x.max(cell.x);
            bounds.min_y = bounds.min_y.min(cell.y);
            bounds.max_y = bounds.max_y.max(cell.y);
        }
        Some(bounds)
    }

    fn width(self) -> f32 {
        (self.max_x - self.min_x + 1) as f32
    }

    fn height(self) -> f32 {
        (self.max_y - self.min_y + 1) as f32
    }

    fn center_x(self) -> f32 {
        (self.min_x + self.max_x) as f32 / 2.0
    }

    fn center_y(self) -> f32 {
        (self.min_y + self.max_y) as f32 / 2.0
    }
}

#[derive(Clone, Copy, Debug)]
struct View {
    center_x: f32,
    center_y: f32,
    zoom: f32,
}

impl View {
    fn fit(bounds: Bounds) -> Self {
        let zoom = ((WIDTH as f32 - 80.0) / bounds.width())
            .min((HEIGHT as f32 - 80.0) / bounds.height())
            .clamp(1.0, 64.0);
        Self {
            center_x: bounds.center_x(),
            center_y: bounds.center_y(),
            zoom,
        }
    }

    fn clamp(&mut self, bounds: Bounds) {
        let half_w = WIDTH as f32 / (2.0 * self.zoom);
        let half_h = HEIGHT as f32 / (2.0 * self.zoom);
        let min_x = bounds.min_x as f32 - 0.5;
        let max_x = bounds.max_x as f32 + 0.5;
        let min_y = bounds.min_y as f32 - 0.5;
        let max_y = bounds.max_y as f32 + 0.5;

        if max_x - min_x <= half_w * 2.0 {
            self.center_x = (min_x + max_x) * 0.5;
        } else {
            self.center_x = self.center_x.clamp(min_x + half_w, max_x - half_w);
        }
        if max_y - min_y <= half_h * 2.0 {
            self.center_y = (min_y + max_y) * 0.5;
        } else {
            self.center_y = self.center_y.clamp(min_y + half_h, max_y - half_h);
        }
    }
}

pub fn run(cells: &[Cell], frames: Option<usize>) -> std::result::Result<(), Box<dyn Error>> {
    let Some(bounds) = Bounds::new(cells) else {
        return Err("no exterior cells to draw".into());
    };
    let max_refs = cells.iter().map(|cell| cell.refs).max().unwrap_or(0);
    let densest = cells.iter().max_by_key(|cell| cell.refs).copied();
    let mut view = View::fit(bounds);

    let mut window = Window::new(
        "Open CE - Tamriel cell map",
        WIDTH,
        HEIGHT,
        WindowOptions {
            resize: false,
            ..WindowOptions::default()
        },
    )?;
    window.set_target_fps(60);

    let mut buffer = vec![BACKGROUND; WIDTH * HEIGHT];
    let mut title = String::new();
    let mut drawn = 0usize;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let mut moved = false;
        if window.is_key_pressed(Key::Left, KeyRepeat::No) {
            view.center_x -= 1.0;
            moved = true;
        }
        if window.is_key_pressed(Key::Right, KeyRepeat::No) {
            view.center_x += 1.0;
            moved = true;
        }
        if window.is_key_pressed(Key::Up, KeyRepeat::No) {
            view.center_y += 1.0;
            moved = true;
        }
        if window.is_key_pressed(Key::Down, KeyRepeat::No) {
            view.center_y -= 1.0;
            moved = true;
        }
        if window.is_key_pressed(Key::Q, KeyRepeat::No) {
            view.zoom = (view.zoom / 1.25).max(1.0);
            moved = true;
        }
        if window.is_key_pressed(Key::E, KeyRepeat::No) {
            view.zoom = (view.zoom * 1.25).min(64.0);
            moved = true;
        }
        if window.is_key_pressed(Key::R, KeyRepeat::No) {
            view = View::fit(bounds);
            moved = true;
        }
        if moved {
            view.clamp(bounds);
        }

        draw(&mut buffer, cells, &view, max_refs, densest);
        let next_title = format!(
            "Open CE Tamriel map - {} cells - center ({:.0}, {:.0}) - zoom {:.1} - arrows pan, Q/E zoom, R reset, Esc quit",
            cells.len(),
            view.center_x,
            view.center_y,
            view.zoom
        );
        if next_title != title {
            window.set_title(&next_title);
            title = next_title;
        }
        window.update_with_buffer(&buffer, WIDTH, HEIGHT)?;
        drawn += 1;
        if let Some(limit) = frames {
            if limit > 0 && drawn >= limit {
                break;
            }
        }
    }

    Ok(())
}

fn draw(buffer: &mut [u32], cells: &[Cell], view: &View, max_refs: u32, densest: Option<Cell>) {
    buffer.fill(BACKGROUND);
    let half_w = WIDTH as f32 / (2.0 * view.zoom);
    let half_h = HEIGHT as f32 / (2.0 * view.zoom);
    let left = view.center_x - half_w;
    let top = view.center_y + half_h;
    let size = view.zoom.round().max(1.0) as i32;

    for cell in cells {
        let x = ((cell.x as f32 - left) * view.zoom).round() as i32;
        let y = ((top - cell.y as f32) * view.zoom).round() as i32;
        if x + size < 0 || y + size < 0 || x >= WIDTH as i32 || y >= HEIGHT as i32 {
            continue;
        }
        fill_rect(buffer, x, y, size, size, cell_color(cell.refs, max_refs));
        if size >= 4 {
            draw_rect(buffer, x, y, size, size, GRID);
        }
    }

    if let Some(cell) = densest {
        let x = ((cell.x as f32 - left) * view.zoom).round() as i32;
        let y = ((top - cell.y as f32) * view.zoom).round() as i32;
        let marker = size.max(6);
        draw_rect(buffer, x, y, marker, marker, 0x00FFCC66);
        draw_rect(buffer, x - 1, y - 1, marker + 2, marker + 2, 0x00FFCC66);
    }
}

fn cell_color(refs: u32, max_refs: u32) -> u32 {
    if refs == 0 {
        return 0x0010_1814;
    }
    let t = ((refs as f32 + 1.0).ln() / (max_refs.max(1) as f32 + 1.0).ln()).clamp(0.0, 1.0);
    let red = (40.0 + 200.0 * t) as u32;
    let green = (80.0 + 120.0 * (1.0 - t)) as u32;
    let blue = (50.0 + 40.0 * (1.0 - t)) as u32;
    (red << 16) | (green << 8) | blue
}

fn fill_rect(buffer: &mut [u32], x: i32, y: i32, width: i32, height: i32, color: u32) {
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + width).min(WIDTH as i32);
    let y1 = (y + height).min(HEIGHT as i32);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    for row in y0..y1 {
        let start = row as usize * WIDTH + x0 as usize;
        let end = row as usize * WIDTH + x1 as usize;
        buffer[start..end].fill(color);
    }
}

fn draw_rect(buffer: &mut [u32], x: i32, y: i32, width: i32, height: i32, color: u32) {
    if width <= 0 || height <= 0 {
        return;
    }
    fill_rect(buffer, x, y, width, 1, color);
    fill_rect(buffer, x, y + height - 1, width, 1, color);
    fill_rect(buffer, x, y, 1, height, color);
    fill_rect(buffer, x + width - 1, y, 1, height, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_cover_cells() {
        let cells = [
            Cell {
                x: -2,
                y: 4,
                refs: 1,
            },
            Cell {
                x: 5,
                y: -3,
                refs: 2,
            },
        ];
        let bounds = Bounds::new(&cells).unwrap();
        assert_eq!(bounds.min_x, -2);
        assert_eq!(bounds.max_x, 5);
        assert_eq!(bounds.min_y, -3);
        assert_eq!(bounds.max_y, 4);
    }

    #[test]
    fn empty_map_has_no_bounds() {
        assert!(Bounds::new(&[]).is_none());
    }

    #[test]
    fn empty_cells_are_dark() {
        assert_eq!(cell_color(0, 10), 0x0010_1814);
    }

    fn real_plugin() -> Option<std::path::PathBuf> {
        let root = std::env::var_os("OCE_SKYRIM_SE").map(std::path::PathBuf::from)?;
        let data = if root.join("Data").is_dir() {
            root.join("Data")
        } else {
            root
        };
        let path = data.join("Skyrim.esm");
        path.is_file().then_some(path)
    }

    #[test]
    fn real_map_renders_if_present() {
        let Some(path) = real_plugin() else {
            eprintln!("skipping real map test: game data not found");
            return;
        };
        let plugin = Plugin::open(path).unwrap();
        let cells = build_cells(&plugin).unwrap();
        assert!(!cells.is_empty());
        let bounds = Bounds::new(&cells).unwrap();
        let view = View::fit(bounds);
        let max_refs = cells.iter().map(|cell| cell.refs).max().unwrap_or(0);
        let densest = cells.iter().max_by_key(|cell| cell.refs).copied();
        let mut buffer = vec![BACKGROUND; WIDTH * HEIGHT];
        draw(&mut buffer, &cells, &view, max_refs, densest);
        let painted = buffer.iter().filter(|&&pixel| pixel != BACKGROUND).count();
        assert!(painted > 1000, "only {painted} pixels were painted");
    }

    #[test]
    fn draw_changes_pixels() {
        let cells = [Cell {
            x: 0,
            y: 0,
            refs: 5,
        }];
        let view = View {
            center_x: 0.0,
            center_y: 0.0,
            zoom: 16.0,
        };
        let mut buffer = vec![BACKGROUND; WIDTH * HEIGHT];
        draw(&mut buffer, &cells, &view, 5, Some(cells[0]));
        assert!(buffer.iter().any(|&pixel| pixel != BACKGROUND));
    }
}
