"""Heimdall logomark: helmeted H with wings, halo, central sword, and
circuit-trace flourishes.

All geometry is computed from a few named ratios so the mark scales
cleanly. The layout, in coordinate space:

                       sword tip
                          ▲
        ────  ╲─╲─╲      │      ╱─╱─╱  ────       circuit traces +
                          │                       wings (mirrored)
              ┌───┐       │       ┌───┐
              │   ╲───────┼───────╱   │           helmet H with
              │       ▼───┼───▼       │           visor notch
              │           │           │
              │           ▼           │
              └───────────┴───────────┘
                          ▼
                     sword pommel

The arc/halo sits behind the helmet; the wordmark "HEIMDALL" lives in
`logo.py` below the mark.
"""

import math

from heimdall_logo.svg import SvgDocument


class Logomark:
    """Generates the Heimdall helmet-H logomark SVG."""

    def __init__(
        self,
        # Canvas (unitless coordinate space).
        size: float = 240,
        # Helmet H proportions (fractions of `size`).
        helmet_width: float = 0.46,
        helmet_height: float = 0.52,
        bar_thickness: float = 0.08,   # stroke width as a fraction of size
        visor_drop: float = 0.08,      # how far the V-notch descends from the crossbar
        # Sword.
        sword_length: float = 0.86,    # total height of the sword (fraction of size)
        sword_thickness: float = 0.05,
        # Wings: horizontal feathers fanning slightly upward, mirrored per side.
        wing_count: int = 5,
        wing_inner_x: float = 0.26,    # x distance from center at the inner (helmet-facing) end
        wing_outer_x: float = 0.50,    # x distance from center at the outer (tip) end
        wing_top_y: float = 0.20,      # y distance above center where the topmost feather sits
        wing_bottom_y: float = 0.04,   # y distance above center where the bottommost feather sits
        wing_tilt: float = 0.06,       # how much each feather rises from inner to outer
        # Halo arc.
        halo_radius: float = 0.46,
        halo_thickness: float = 0.015,
        halo_arc_degrees: float = 220, # how much of the circle is drawn
        # Circuit traces.
        trace_count: int = 2,
        trace_length: float = 0.18,
        trace_node_radius: float = 0.014,
        trace_outer_offset: float = 0.50,
    ):
        self.size = size
        # All ratios captured; the geometry computations live in `_layout`.
        self.helmet_width = helmet_width
        self.helmet_height = helmet_height
        self.bar_thickness = bar_thickness
        self.visor_drop = visor_drop
        self.sword_length = sword_length
        self.sword_thickness = sword_thickness
        self.wing_count = wing_count
        self.wing_inner_x = wing_inner_x
        self.wing_outer_x = wing_outer_x
        self.wing_top_y = wing_top_y
        self.wing_bottom_y = wing_bottom_y
        self.wing_tilt = wing_tilt
        self.halo_radius = halo_radius
        self.halo_thickness = halo_thickness
        self.halo_arc_degrees = halo_arc_degrees
        self.trace_count = trace_count
        self.trace_length = trace_length
        self.trace_node_radius = trace_node_radius
        self.trace_outer_offset = trace_outer_offset

    # ------------------------------------------------------------------
    # Layout helpers

    @property
    def cx(self) -> float:
        return self.size / 2

    @property
    def cy(self) -> float:
        return self.size / 2

    def _abs(self, frac: float) -> float:
        return frac * self.size

    # ------------------------------------------------------------------
    # Element draws

    def _draw_halo(self, doc: SvgDocument, parent) -> None:
        r = self._abs(self.halo_radius)
        half = self.halo_arc_degrees / 2
        # Arc opening at the bottom so the wordmark beneath reads cleanly.
        start_angle = 270 - half  # measured from 12 o'clock = -90
        end_angle = 270 + half
        x0 = self.cx + r * math.cos(math.radians(start_angle))
        y0 = self.cy + r * math.sin(math.radians(start_angle))
        x1 = self.cx + r * math.cos(math.radians(end_angle))
        y1 = self.cy + r * math.sin(math.radians(end_angle))
        large = 1 if self.halo_arc_degrees > 180 else 0
        d = f"M {x0:.3f},{y0:.3f} A {r:.3f},{r:.3f} 0 {large} 1 {x1:.3f},{y1:.3f}"
        doc.path(
            parent,
            d,
            fill="none",
            stroke="url(#heimdall-grad)",
            stroke_width=str(self._abs(self.halo_thickness)),
            stroke_linecap="round",
            opacity="0.55",
        )

    def _draw_helmet(self, doc: SvgDocument, parent) -> None:
        w = self._abs(self.helmet_width)
        h = self._abs(self.helmet_height)
        t = self._abs(self.bar_thickness)
        v = self._abs(self.visor_drop)

        left_x = self.cx - w / 2
        right_x = self.cx + w / 2
        top_y = self.cy - h / 2
        bot_y = self.cy + h / 2
        bar_y = self.cy  # crossbar (visor) at mid height

        stroke = "url(#heimdall-grad)"
        common = dict(
            fill="none",
            stroke=stroke,
            stroke_width=str(t),
            stroke_linecap="round",
            stroke_linejoin="round",
        )

        # Left + right helmet bars.
        doc.line(parent, left_x, top_y, left_x, bot_y, **common)
        doc.line(parent, right_x, top_y, right_x, bot_y, **common)

        # V-notched crossbar / visor: two diagonals meeting at the center.
        notch_y = bar_y + v
        doc.path(
            parent,
            f"M {left_x:.3f},{bar_y:.3f} "
            f"L {self.cx:.3f},{notch_y:.3f} "
            f"L {right_x:.3f},{bar_y:.3f}",
            **common,
        )

    def _draw_sword(self, doc: SvgDocument, parent) -> None:
        l = self._abs(self.sword_length)
        t = self._abs(self.sword_thickness)
        top_y = self.cy - l / 2
        bot_y = self.cy + l / 2

        common = dict(
            fill="none",
            stroke="url(#heimdall-grad)",
            stroke_width=str(t),
            stroke_linecap="square",
        )

        # Vertical blade.
        doc.line(parent, self.cx, top_y, self.cx, bot_y, **common)
        # Pointed tip: small triangle filled with the gradient.
        tip_h = t * 1.8
        doc.path(
            parent,
            f"M {self.cx - t * 0.7:.3f},{top_y:.3f} "
            f"L {self.cx:.3f},{top_y - tip_h:.3f} "
            f"L {self.cx + t * 0.7:.3f},{top_y:.3f} Z",
            fill="url(#heimdall-grad)",
            stroke="none",
        )

    def _draw_wings(self, doc: SvgDocument, parent) -> None:
        # Feathers are stacked vertically and tilt slightly upward as they
        # extend outward, so the wing reads as horizontal layers rather than
        # a radial fan.
        inner_dx = self._abs(self.wing_inner_x)
        outer_dx = self._abs(self.wing_outer_x)
        top_dy = self._abs(self.wing_top_y)
        bot_dy = self._abs(self.wing_bottom_y)
        tilt = self._abs(self.wing_tilt)
        t = self._abs(self.bar_thickness * 0.5)

        n = self.wing_count
        for side in (-1, 1):
            for i in range(n):
                # 0 -> bottommost (closest to center), n-1 -> topmost (longest).
                ratio = i / max(n - 1, 1)
                # Top feathers reach further outward to suggest fanning.
                outer_reach = inner_dx + (outer_dx - inner_dx) * (0.55 + 0.45 * ratio)
                inner_y = self.cy - (bot_dy + (top_dy - bot_dy) * ratio)
                outer_y = inner_y - tilt
                ix = self.cx + side * inner_dx
                ox = self.cx + side * outer_reach
                doc.line(
                    parent,
                    ix,
                    inner_y,
                    ox,
                    outer_y,
                    fill="none",
                    stroke="url(#heimdall-grad)",
                    stroke_width=str(t),
                    stroke_linecap="round",
                )

    def _draw_traces(self, doc: SvgDocument, parent) -> None:
        t = self._abs(self.bar_thickness * 0.32)
        length = self._abs(self.trace_length)
        node_r = self._abs(self.trace_node_radius)
        outer = self._abs(self.trace_outer_offset)

        for side in (-1, 1):
            for i in range(self.trace_count):
                # Stagger above and below center.
                offset = (i - (self.trace_count - 1) / 2) * self._abs(0.05)
                y = self.cy + offset
                x_end = self.cx + side * outer
                x_start = x_end - side * length
                doc.line(
                    parent,
                    x_start,
                    y,
                    x_end,
                    y,
                    fill="none",
                    stroke="url(#heimdall-grad)",
                    stroke_width=str(t),
                    stroke_linecap="round",
                )
                # Terminal node on the outer end.
                doc.circle(
                    parent,
                    x_end,
                    y,
                    node_r,
                    fill="url(#heimdall-grad)",
                    stroke="none",
                )

    # ------------------------------------------------------------------
    # Public entry points

    def draw_into(self, doc: SvgDocument, parent) -> None:
        """Render the logomark into an existing SVG group."""
        # Order matters for visual stacking.
        self._draw_halo(doc, parent)
        self._draw_traces(doc, parent)
        self._draw_wings(doc, parent)
        self._draw_helmet(doc, parent)
        self._draw_sword(doc, parent)

    def render(self, background: str | None = None, margin: float = 12.0) -> SvgDocument:
        """Render the logomark as a standalone SVG document."""
        from heimdall_logo.colors import HeimdallPalette

        side = self.size + margin * 2
        doc = SvgDocument(side, side, viewbox=(0, 0, side, side))

        # Gradient defined once and referenced by every stroke. Spans the
        # full canvas in user space so vertical/horizontal lines still
        # receive both color stops.
        palette = HeimdallPalette()
        doc.linear_gradient(
            "heimdall-grad",
            palette.gradient_stops(),
            x1=0,
            y1=0,
            x2=side,
            y2=side,
        )

        if background is not None:
            doc.rect(doc.root, 0, 0, side, side, fill=background, rx=12, ry=12)

        wrapper = doc.group(transform=f"translate({margin},{margin})")
        self.draw_into(doc, wrapper)
        return doc
