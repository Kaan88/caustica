//! The "Compiling shaders 14/22" screen shown while the pipelines compile on a cold driver
//! cache, like a game's: text above a bar, drawn by both backends with clear rects only (no
//! pipeline is ready yet). A warm cache finishes before the first progress frame is due.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// One colour (display-encoded RGB: the swap chains start UNORM) and its rects (x, y, w, h).
pub type Layer = ([f32; 3], Vec<[u32; 4]>);

/// 5×7 glyphs for the text this screen needs ('#' lit), by row.
fn glyph(c: char) -> [&'static str; 7] {
    match c {
        'C' => [
            " ### ", "#   #", "#    ", "#    ", "#    ", "#   #", " ### ",
        ],
        'o' => [
            "     ", "     ", " ### ", "#   #", "#   #", "#   #", " ### ",
        ],
        'm' => [
            "     ", "     ", "## # ", "# # #", "# # #", "# # #", "# # #",
        ],
        'p' => [
            "     ", "     ", "#### ", "#   #", "#### ", "#    ", "#    ",
        ],
        'i' => [
            "  #  ", "     ", " ##  ", "  #  ", "  #  ", "  #  ", " ### ",
        ],
        'l' => [
            " ##  ", "  #  ", "  #  ", "  #  ", "  #  ", "  #  ", " ### ",
        ],
        'n' => [
            "     ", "     ", "# ## ", "##  #", "#   #", "#   #", "#   #",
        ],
        'g' => [
            "     ", "     ", " ####", "#   #", " ####", "    #", " ### ",
        ],
        's' => [
            "     ", "     ", " ####", "#    ", " ### ", "    #", "#### ",
        ],
        'h' => [
            "#    ", "#    ", "# ## ", "##  #", "#   #", "#   #", "#   #",
        ],
        'a' => [
            "     ", "     ", " ### ", "    #", " ####", "#   #", " ####",
        ],
        'd' => [
            "    #", "    #", " ## #", "#  ##", "#   #", "#   #", " ####",
        ],
        'e' => [
            "     ", "     ", " ### ", "#   #", "#####", "#    ", " ### ",
        ],
        'r' => [
            "     ", "     ", "# ## ", "##  #", "#    ", "#    ", "#    ",
        ],
        '/' => [
            "    #", "   # ", "   # ", "  #  ", " #   ", " #   ", "#    ",
        ],
        '0' => [
            " ### ", "#   #", "#  ##", "# # #", "##  #", "#   #", " ### ",
        ],
        '1' => [
            "  #  ", " ##  ", "  #  ", "  #  ", "  #  ", "  #  ", " ### ",
        ],
        '2' => [
            " ### ", "#   #", "    #", "   # ", "  #  ", " #   ", "#####",
        ],
        '3' => [
            "#####", "   # ", "  #  ", "   # ", "    #", "#   #", " ### ",
        ],
        '4' => [
            "   # ", "  ## ", " # # ", "#  # ", "#####", "   # ", "   # ",
        ],
        '5' => [
            "#####", "#    ", "#### ", "    #", "    #", "#   #", " ### ",
        ],
        '6' => [
            "  ## ", " #   ", "#    ", "#### ", "#   #", "#   #", " ### ",
        ],
        '7' => [
            "#####", "    #", "   # ", "  #  ", " #   ", " #   ", " #   ",
        ],
        '8' => [
            " ### ", "#   #", "#   #", " ### ", "#   #", "#   #", " ### ",
        ],
        '9' => [
            " ### ", "#   #", "#   #", " ####", "    #", "   # ", " ##  ",
        ],
        _ => ["     "; 7],
    }
}

/// The background colour the frame is cleared to first.
pub const BACKGROUND: [f32; 3] = [0.07, 0.07, 0.08];

/// The bar track, its fill and the text for a `w`×`h` frame, `done` of `total` compiled.
/// Every rect lies inside the frame (clear rects must).
pub fn layout(w: u32, h: u32, done: usize, total: usize) -> [Layer; 3] {
    let px = (h / 200).max(2);
    let text = format!("Compiling shaders {done}/{total}");
    let tw = text.chars().count() as i64 * 6 * px as i64;
    let (cx, cy) = (w as i64 / 2, h as i64 / 2);
    let (bar_w, bar_h) = ((w / 2).max(1), 3 * px);
    let bar_x = cx - bar_w as i64 / 2;
    let bar_y = cy + 4 * px as i64;
    let (x0, y0) = (cx - tw / 2, cy - 9 * px as i64);
    let mut glyphs = Vec::new();
    for (k, ch) in text.chars().enumerate() {
        for (row, bits) in glyph(ch).iter().enumerate() {
            // One rect per run of lit pixels in a row.
            let bits = bits.as_bytes();
            let mut col = 0;
            while col < bits.len() {
                if bits[col] != b'#' {
                    col += 1;
                    continue;
                }
                let start = col;
                while col < bits.len() && bits[col] == b'#' {
                    col += 1;
                }
                glyphs.push((
                    x0 + ((k * 6 + start) as i64) * px as i64,
                    y0 + row as i64 * px as i64,
                    (col - start) as u32 * px,
                    px,
                ));
            }
        }
    }
    let fill = (bar_w as u64 * done as u64 / total.max(1) as u64) as u32;
    let inside = |rects: Vec<(i64, i64, u32, u32)>| -> Vec<[u32; 4]> {
        rects
            .into_iter()
            .filter(|&(x, y, rw, rh)| {
                x >= 0
                    && y >= 0
                    && rw > 0
                    && rh > 0
                    && x + rw as i64 <= w as i64
                    && y + rh as i64 <= h as i64
            })
            .map(|(x, y, rw, rh)| [x as u32, y as u32, rw, rh])
            .collect()
    };
    [
        (
            [0.18, 0.18, 0.2],
            inside(vec![(bar_x, bar_y, bar_w, bar_h)]),
        ),
        (
            [0.95, 0.55, 0.18],
            inside(vec![(bar_x, bar_y, fill, bar_h)]),
        ),
        ([0.85, 0.85, 0.87], inside(glyphs)),
    ]
}

/// Waits for `done` to reach `total`, calling `draw(done)` each time it changes once 250 ms
/// have passed. Returns whether the screen was shown.
/// Windows marks a window "not responding" when its thread hasn't looked at its message queue
/// for 5 s, which a cold driver compile exceeds. A peek that removes nothing counts as looking,
/// without dispatching messages into winit, whose callback this runs inside.
fn keep_responsive() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_NOREMOVE, PeekMessageW};
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
    }
}
pub fn watch(done: &AtomicUsize, total: usize, mut draw: impl FnMut(usize)) -> bool {
    let start = Instant::now();
    let mut shown = None;
    loop {
        let n = done.load(Ordering::Relaxed);
        if n >= total {
            break;
        }
        if start.elapsed() > Duration::from_millis(250) && shown != Some(n) {
            shown = Some(n);
            draw(n);
        }
        keep_responsive();
        std::thread::sleep(Duration::from_millis(16));
    }
    if shown.is_some() {
        crate::startup(&format!(
            "compiled {total} shaders with the progress screen"
        ));
    }
    shown.is_some()
}

#[cfg(test)]
mod tests {
    #[test]
    fn progress_rects_stay_inside_the_frame() {
        for (w, h) in [(1, 1), (64, 48), (800, 600), (3440, 1440)] {
            for layer in super::layout(w, h, 14, 22) {
                for [x, y, rw, rh] in layer.1 {
                    assert!(x + rw <= w && y + rh <= h);
                }
            }
        }
        // The text is there at a normal size, and the fill grows with progress.
        let [_, fill, text] = super::layout(800, 600, 11, 22);
        assert!(text.1.len() > 50);
        assert_eq!(fill.1[0][2], 200);
    }
}
