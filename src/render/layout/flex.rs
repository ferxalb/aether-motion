//! Flex container layout: content-sized dimensions and placement of in-flow children.

/// A flex item before its main size grows into the free space.
pub(in crate::render) struct FlexItem {
    idx: usize,
    main: f32,
    cross: Option<f32>,
    grow: f32,
}

impl<'a> Renderer<'a> {
    /// Content-sized dimensions of a flex container, padding included.
    pub(in crate::render) fn flex_intrinsic(&mut self, idx: usize, ps: f32) -> Result<Option<(f32, f32)>> {
        let composition: &'a Composition = self.composition;
        let style = &composition.elements[idx].style;
        let kids: Vec<usize> = composition.elements[idx]
            .children
            .iter()
            .copied()
            .filter(|&child| in_flow(composition, child))
            .collect();
        if kids.is_empty() {
            return Ok(None);
        }
        let (mut main, mut cross) = (0.0_f32, 0.0_f32);
        for &kid in &kids {
            let (w, h) = self.item_size(kid, ps, (0.0, 0.0))?;
            let (m, c) = if style.column { (h, w) } else { (w, h) };
            main += m.unwrap_or(0.0);
            cross = cross.max(c.unwrap_or(0.0));
        }
        main += style.gap * ps * (kids.len() - 1) as f32;
        let [top, right, bottom, left] = style.padding.map(|p| p * ps);
        Ok(Some(if style.column {
            (cross + left + right, main + top + bottom)
        } else {
            (main + left + right, cross + top + bottom)
        }))
    }

    /// Places the in-flow children of a flex container inside `content`.
    pub(in crate::render) fn flex_layout(&mut self, idx: usize, content: Rect, ps: f32) -> Result<Vec<(usize, Rect)>> {
        let composition: &'a Composition = self.composition;
        let style = &composition.elements[idx].style;
        let kids: Vec<usize> = composition.elements[idx]
            .children
            .iter()
            .copied()
            .filter(|&child| in_flow(composition, child))
            .collect();
        if kids.is_empty() {
            return Ok(Vec::new());
        }
        let column = style.column;
        let (main_space, cross_space) = if column {
            (content.h, content.w)
        } else {
            (content.w, content.h)
        };
        let gap = style.gap * ps;
        let stretch = !matches!(
            style.align_items.as_deref(),
            Some("flex-start" | "start" | "center" | "flex-end" | "end")
        );
        let mut items = Vec::with_capacity(kids.len());
        for &kid in &kids {
            let (w, h) = self.item_size(kid, ps, (content.w, content.h))?;
            let (main, cross) = if column { (h, w) } else { (w, h) };
            let kid_style = &composition.elements[kid].style;
            items.push(FlexItem {
                idx: kid,
                main: if kid_style.flex_basis_zero {
                    0.0
                } else {
                    main.unwrap_or(0.0)
                },
                cross,
                grow: kid_style.flex_grow,
            });
        }
        let gaps = gap * (items.len() - 1) as f32;
        let free = main_space - items.iter().map(|item| item.main).sum::<f32>() - gaps;
        let total_grow: f32 = items.iter().map(|item| item.grow).sum();
        let free = if free > 0.0 && total_grow > 0.0 {
            for item in &mut items {
                item.main += free * item.grow / total_grow;
            }
            0.0
        } else {
            free
        };
        let count = items.len() as f32;
        let spare = free.max(0.0);
        let (start, between) = match style.justify_content.as_deref() {
            Some("center") => (free / 2.0, 0.0),
            Some("flex-end" | "end") => (free, 0.0),
            Some("space-between") if items.len() > 1 => (0.0, spare / (count - 1.0)),
            Some("space-around") => (spare / count / 2.0, spare / count),
            Some("space-evenly") => (spare / (count + 1.0), spare / (count + 1.0)),
            _ => (0.0, 0.0),
        };
        let mut position = start;
        let mut placed = Vec::with_capacity(items.len());
        for item in items {
            let cross = item
                .cross
                .unwrap_or(if stretch { cross_space } else { 0.0 });
            let offset = match style.align_items.as_deref() {
                Some("center") => (cross_space - cross) / 2.0,
                Some("flex-end" | "end") => cross_space - cross,
                _ => 0.0,
            };
            let rect = if column {
                Rect {
                    x: content.x + offset,
                    y: content.y + position,
                    w: cross,
                    h: item.main,
                }
            } else {
                Rect {
                    x: content.x + position,
                    y: content.y + offset,
                    w: item.main,
                    h: cross,
                }
            };
            placed.push((item.idx, rect));
            position += item.main + gap + between;
        }
        Ok(placed)
    }
}
