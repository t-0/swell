//! How the waveform is drawn with text: each cell is split into a small grid
//! of pixels (`grid()`), and the pixels the waveform covers pick a glyph.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WaveRender {
    /// Two pixels per cell (1×2) with `▀ ▄ █`.
    #[default]
    Blocks,
    /// Four pixels per cell (2×2) with the quadrant characters.
    Quads,
    /// Six pixels per cell (2×3) with the sextant characters.
    Sextants,
    /// Eight dots per cell (2×4) with Braille patterns.
    Braille,
    /// Three rows per cell (1×3) with plain ASCII.
    Ascii,
}

impl WaveRender {
    pub const ALL: [WaveRender; 5] = [
        WaveRender::Blocks,
        WaveRender::Quads,
        WaveRender::Sextants,
        WaveRender::Braille,
        WaveRender::Ascii,
    ];

    pub fn name(self) -> &'static str {
        match self {
            WaveRender::Blocks => "blocks",
            WaveRender::Quads => "quads",
            WaveRender::Sextants => "sextants",
            WaveRender::Braille => "braille",
            WaveRender::Ascii => "ascii",
        }
    }

    pub fn parse(s: &str) -> Option<WaveRender> {
        let s = s.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|r| r.name() == s)
    }

    pub fn next(self) -> WaveRender {
        let i = Self::ALL.iter().position(|&r| r == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Pixels per cell: (columns, rows).
    pub fn grid(self) -> (usize, usize) {
        match self {
            WaveRender::Blocks => (1, 2),
            WaveRender::Quads => (2, 2),
            WaveRender::Sextants => (2, 3),
            WaveRender::Braille => (2, 4),
            WaveRender::Ascii => (1, 3),
        }
    }

    /// The glyph for a mask of lit pixels, bit `row * cols + col`.
    /// Mask 0 is never asked for (the caller draws the background).
    pub fn glyph(self, mask: usize) -> char {
        match self {
            WaveRender::Blocks => [' ', '▀', '▄', '█'][mask & 3],
            WaveRender::Quads => QUADS[mask & 15],
            WaveRender::Sextants => sextant(mask & 63),
            WaveRender::Braille => braille(mask & 255),
            WaveRender::Ascii => match mask & 7 {
                0 => ' ',
                1 => '\'',
                2 => '-',
                4 => '.',
                _ => '|',
            },
        }
    }
}

/// Quadrants: bit 0 top left, 1 top right, 2 bottom left, 3 bottom right.
const QUADS: [char; 16] = [
    ' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█',
];

/// The sextant for a 2×3 mask (bit 0 top left … 5 bottom right). Unicode
/// leaves out the four shapes that already had characters.
fn sextant(mask: usize) -> char {
    match mask {
        0 => ' ',
        21 => '▌',
        42 => '▐',
        63 => '█',
        m => {
            let skipped = (m > 21) as u32 + (m > 42) as u32;
            char::from_u32(0x1FB00 + m as u32 - 1 - skipped).unwrap_or('?')
        }
    }
}

/// The Braille pattern for a 2×4 mask in row order.
fn braille(mask: usize) -> char {
    // dot bit for (row, col), Unicode order: 1 2 3 7 down the left, 4 5 6 8 down the right
    const DOT: [[u32; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
    let mut bits = 0;
    for (r, row) in DOT.iter().enumerate() {
        for (c, &d) in row.iter().enumerate() {
            if mask >> (r * 2 + c) & 1 == 1 {
                bits |= d;
            }
        }
    }
    char::from_u32(0x2800 + bits).unwrap_or(' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_cycle() {
        for r in WaveRender::ALL {
            assert_eq!(WaveRender::parse(r.name()), Some(r));
        }
        assert_eq!(WaveRender::parse(" Sextants "), Some(WaveRender::Sextants));
        assert_eq!(WaveRender::parse("nope"), None);
        let mut r = WaveRender::default();
        for _ in 0..WaveRender::ALL.len() {
            r = r.next();
        }
        assert_eq!(r, WaveRender::Blocks);
    }

    #[test]
    fn glyphs() {
        assert_eq!(WaveRender::Blocks.glyph(1), '▀');
        assert_eq!(WaveRender::Blocks.glyph(3), '█');
        assert_eq!(WaveRender::Quads.glyph(0b1010), '▐');
        assert_eq!(sextant(1), '\u{1FB00}');
        assert_eq!(sextant(22), '\u{1FB14}');
        assert_eq!(sextant(62), '\u{1FB3B}');
        assert_eq!(braille(0b11111111), '⣿');
        assert_eq!(braille(0b01000000), '⡀');
        assert_eq!(braille(0b00000010), '⠈');
    }
}
