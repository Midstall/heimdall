"""Render the 'HEIMDALL' wordmark via fontforge for path-accurate output.
Same mechanism as midstall-logo. Font path comes from the
HEIMDALL_LOGOTYPE_FONT_FILE env var or an explicit argument."""

import os

import fontforge

from heimdall_logo.svg import SvgDocument


class FontLoader:
    def __init__(self, font_path: str | None = None):
        if font_path is None:
            font_path = os.environ["HEIMDALL_LOGOTYPE_FONT_FILE"]
        self._font = fontforge.open(font_path)
        self._units_per_em = self._font.em

    def _glyph_to_svg_path(self, glyph) -> tuple[str, float]:
        layer = glyph.layers[glyph.activeLayer]
        parts: list[str] = []

        for contour in layer:
            points = list(contour)
            n = len(points)
            if n == 0:
                continue

            parts.append(f"M {points[0].x},{-points[0].y}")
            i = 1
            while i < n:
                pt = points[i]
                if pt.on_curve:
                    parts.append(f"L {pt.x},{-pt.y}")
                    i += 1
                else:
                    cp1 = points[i]
                    cp2 = points[(i + 1) % n]
                    end = points[(i + 2) % n]
                    parts.append(
                        f"C {cp1.x},{-cp1.y} {cp2.x},{-cp2.y} {end.x},{-end.y}"
                    )
                    if (i + 2) % n < i:
                        break
                    i += 3

            if contour.closed:
                parts.append("Z")

        return " ".join(parts), float(glyph.width)

    def text_to_paths(
        self,
        text: str,
        font_size: float,
        letter_spacing: float = 0,
    ) -> tuple[list[tuple[str, float]], float, float]:
        scale = font_size / self._units_per_em
        ascent = self._font.ascent * scale

        paths: list[tuple[str, float]] = []
        cursor_x = 0.0
        for ch in text:
            if ch == " ":
                cursor_x += self._font["space"].width * scale + letter_spacing
                continue
            glyph = self._font[ch]
            path_d, advance = self._glyph_to_svg_path(glyph)
            paths.append((path_d, cursor_x))
            cursor_x += advance * scale + letter_spacing

        if text and not text.endswith(" "):
            cursor_x -= letter_spacing
        return paths, cursor_x, ascent

    def render_text_group(
        self,
        doc: SvgDocument,
        parent,
        text: str,
        x: float,
        y: float,
        font_size: float,
        fill_color: str,
        letter_spacing: float = 0,
    ) -> float:
        scale = font_size / self._units_per_em
        paths, total_width, ascent = self.text_to_paths(
            text, font_size, letter_spacing
        )
        for path_d, x_offset in paths:
            doc.path(
                parent,
                path_d,
                fill=fill_color,
                transform=f"translate({x + x_offset},{y + ascent}) scale({scale},{scale})",
            )
        return total_width
