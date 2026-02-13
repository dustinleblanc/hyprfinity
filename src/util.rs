pub(crate) fn clamp_i32(v: i32, min: i32, max: i32) -> i32 {
    v.max(min).min(max)
}

pub(crate) fn even_floor(v: i32) -> i32 {
    if v <= 2 {
        2
    } else if v % 2 == 0 {
        v
    } else {
        v - 1
    }
}

pub(crate) fn scaled_dimensions(span_width: i32, span_height: i32, scale: f32) -> (i32, i32) {
    let w = (span_width as f32 * scale).round() as i32;
    let h = (span_height as f32 * scale).round() as i32;
    let w = even_floor(clamp_i32(w, 2, span_width));
    let h = even_floor(clamp_i32(h, 2, span_height));
    (w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_dimensions_rounds_and_clamps() {
        assert_eq!(scaled_dimensions(1920, 1080, 0.5), (960, 540));
        assert_eq!(scaled_dimensions(5, 5, 1.0), (4, 4));
        assert_eq!(scaled_dimensions(1920, 1080, 0.0001), (2, 2));
        assert_eq!(scaled_dimensions(1920, 1080, 1.2), (1920, 1080));
    }
}
