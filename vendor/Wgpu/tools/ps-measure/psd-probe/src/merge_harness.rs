
/// For the probe, which build.rs appends to a copy of psd_merge.rs: how
/// one gradient-filled layer drawn from its settings compares with the
/// pixels it stores, inside its shape and clear of a stroke's band.
pub(crate) struct GradientCheck {
    pub(crate) name: String,
    /// Why the merge does not draw it, where it does not.
    pub(crate) undrawn: Option<String>,
    /// Pixels compared, then each plane's largest and root mean square
    /// difference in 8-bit levels and its share within 1 level: color
    /// first, then transparency.
    pub(crate) compared: usize,
    pub(crate) color: [f64; 3],
    pub(crate) shape: [f64; 3],
    /// The start's and end's coordinates before they are taken down to
    /// whole pixels, each with how far it lies from the nearest whole
    /// pixel, in pixels and in epsilons of the size of the numbers it was
    /// made from.
    pub(crate) ends: Vec<[f64; 3]>,
}

/// `region` worn away `times` times by a cross, a pixel kept only where it
/// and its four neighbors are, as scipy's binary_erosion wears it.
fn worn(region: &[bool], width: usize, height: usize, times: usize) -> Vec<bool> {
    let mut now = region.to_vec();
    for _ in 0..times {
        let before = now.clone();
        for y in 0..height {
            for x in 0..width {
                let at = |dx: i64, dy: i64| {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    nx >= 0 && ny >= 0 && (nx as usize) < width && (ny as usize) < height && before[ny as usize * width + nx as usize]
                };
                now[y * width + x] = at(0, 0) && at(-1, 0) && at(1, 0) && at(0, -1) && at(0, 1);
            }
        }
    }
    now
}

/// The largest and root mean square of `apart`, in levels, and its share
/// within 1 level.
fn spread(apart: &[f64]) -> [f64; 3] {
    if apart.is_empty() {
        return [0.0; 3];
    }
    let largest = apart.iter().copied().fold(0.0, f64::max);
    let rms = (apart.iter().map(|a| a * a).sum::<f64>() / apart.len() as f64).sqrt();
    let within = apart.iter().filter(|&&a| a <= 1.0).count() as f64 / apart.len() as f64;
    [largest, rms, within]
}

/// What the merge's parts cost on one document, in seconds, each over all
/// the layers it applies to: reading every visible layer's pixels into
/// canvas planes (object), rasterizing every vector mask over the canvas
/// (mask coverage), drawing every fill that stores no pixels (fill), and
/// one blend of a canvas-sized source onto a canvas-sized backdrop, once
/// with a knockout's backdrop copied as put copies it for every layer and
/// once without, each times the visible layers; then the count of visible
/// layers and of vector masks, and the whole merge's seconds.
pub(crate) fn merge_costs(header: &Header, section: &LayerSection, resources: &Resources, color_data: &[u8], colors: Colors) -> Result<Vec<(String, f64)>, String> {
    let records = records(header, section.info)?;
    let channels = header.mode.colors();
    let m = Merge::whole(header, &section.blocks, resources.icc.as_deref(), colors);
    let visible: Vec<&Record> = records.iter().filter(|r| !r.hidden).collect();
    let started = std::time::Instant::now();
    for record in &visible {
        if record.has_pixels() {
            std::hint::black_box(m.object(record)?);
        }
    }
    let object = started.elapsed().as_secs_f64();
    let started = std::time::Instant::now();
    let mut masks = 0;
    for record in &visible {
        if let Some(data) = record.vector_mask() {
            masks += 1;
            std::hint::black_box(psd_vector::read(data)?.coverage_rows(m.width, m.height, 0, m.height));
        }
    }
    let mask = started.elapsed().as_secs_f64();
    let started = std::time::Instant::now();
    let mut undrawn = 0;
    for record in &visible {
        if !record.has_pixels() && record.has_fill() && std::hint::black_box(m.fill(record)?).is_err() {
            undrawn += 1;
        }
    }
    let fill = started.elapsed().as_secs_f64();
    let pixels = m.pixels();
    let source_color = vec![vec![0.5f32; pixels]; channels];
    let (shape, alpha) = (vec![0.5f32; pixels], vec![0.5f32; pixels]);
    let mut compositor = Compositor::new(vec![vec![0.25f32; pixels]; channels], vec![0.75f32; pixels], false);
    let started = std::time::Instant::now();
    compositor.put(&m, &source_color, &shape, &alpha, b"norm", 0, None);
    let put = started.elapsed().as_secs_f64();
    let started = std::time::Instant::now();
    std::hint::black_box((compositor.color_0.clone(), compositor.alpha_0.clone()));
    let copy = started.elapsed().as_secs_f64();
    let started = std::time::Instant::now();
    merged(header, section, color_data, resources, colors)?;
    let whole = started.elapsed().as_secs_f64();
    let layers = visible.len() as f64;
    Ok(vec![
        ("object".to_string(), object),
        ("mask coverage".to_string(), mask),
        ("fill".to_string(), fill),
        ("put, as each layer copies the knockout backdrop, times the layers".to_string(), put * layers),
        ("of which the copy, times the layers".to_string(), copy * layers),
        ("visible layers".to_string(), layers),
        ("vector masks".to_string(), f64::from(masks)),
        ("fills left undrawn".to_string(), f64::from(undrawn)),
        ("whole merge".to_string(), whole),
    ])
}

/// What bands of `rows` rows cost on one core, in seconds summed over
/// every `every`-th band of the document: each band's whole merge, and,
/// each timed again apart from the merge for the layers the merge draws,
/// the vector masks' coverage of the band where the merge applies them,
/// the effects read, and the fills drawn for layers that store no pixels;
/// then the rest of the merge, the merge less those three, and the count
/// of bands sampled.
pub(crate) fn band_costs(header: &Header, section: &LayerSection, resources: &Resources, colors: Colors, rows: usize, every: usize) -> Result<Vec<(String, f64)>, String> {
    let mut records = records(header, section.info)?;
    let drawn = drawn(&tree(&records), records.len());
    prepare(header, &mut records, &drawn);
    let nodes = tree(&records);
    let whole = Merge::whole(header, &section.blocks, resources.icc.as_deref(), colors);
    let (width, height, channels) = (header.width, header.height, whole.channels);
    let rows = rows.max(1);
    let (mut merge, mut masks, mut effects, mut fills, mut sampled) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0usize);
    let mut undrawn = 0u32;
    for band in (0..height.div_ceil(rows)).step_by(every.max(1)) {
        let top = band * rows;
        let count = rows.min(height - top);
        let mut color = vec![vec![0.0f32; width * count]; channels];
        let mut alpha = vec![0.0f32; width * count];
        let slot = Slot { top, rows: count, color: color.iter_mut().map(|plane| &mut plane[..]).collect(), alpha: &mut alpha };
        let started = std::time::Instant::now();
        merge_band(&whole, &records, &nodes, slot)?;
        merge += started.elapsed().as_secs_f64();
        let m = whole.band(top, count);
        for (record, _) in records.iter().zip(&drawn).filter(|(_, drawn)| **drawn) {
            if let Some(data) = record.vector_mask()
                && (!record.has_pixels() || (!record.has_fill() && record.mask.is_some() && !record.has_real()))
            {
                let started = std::time::Instant::now();
                std::hint::black_box(psd_vector::read(data)?.coverage_rows(width, height, top, top + count));
                masks += started.elapsed().as_secs_f64();
            }
            let started = std::time::Instant::now();
            std::hint::black_box(record.effects()?);
            effects += started.elapsed().as_secs_f64();
            if !record.has_pixels() && record.has_fill() {
                let started = std::time::Instant::now();
                if std::hint::black_box(m.fill(record)?).is_err() {
                    undrawn += 1;
                }
                fills += started.elapsed().as_secs_f64();
            }
        }
        sampled += 1;
    }
    Ok(vec![
        ("merge".to_string(), merge),
        ("vector masks".to_string(), masks),
        ("effects".to_string(), effects),
        ("fills".to_string(), fills),
        ("rest".to_string(), merge - masks - effects - fills),
        ("bands sampled".to_string(), sampled as f64),
        ("fills left undrawn".to_string(), f64::from(undrawn)),
    ])
}

/// The document's layers merged in bands of `rows` rows, for the probe's
/// timing of band sizes: the color planes, the alpha and the warnings.
pub(crate) fn merge_bands(header: &Header, section: &LayerSection, resources: &Resources, colors: Colors, rows: usize) -> Result<(Vec<Vec<f32>>, Vec<f32>, Vec<String>), String> {
    let canvas = merge_in_bands(header, section, resources.icc.as_deref(), colors, rows)?;
    Ok((canvas.color, canvas.alpha, canvas.warnings))
}

/// Each gradient-filled layer drawn from its settings, whether or not it
/// stores pixels, compared with the pixels it stores where it stores them.
pub(crate) fn gradient_checks(header: &Header, section: &LayerSection, resources: &Resources, colors: Colors) -> Result<Vec<GradientCheck>, String> {
    let records = records(header, section.info)?;
    let channels = header.mode.colors();
    let m = Merge::whole(header, &section.blocks, resources.icc.as_deref(), colors);
    let (width, height) = (header.width, header.height);
    let mut out = Vec::new();
    for record in &records {
        let Some((FillKind::Gradient, settings)) = record.fill_settings()? else { continue };
        let mut check = GradientCheck { name: record.name.clone(), undrawn: None, compared: 0, color: [0.0; 3], shape: [0.0; 3], ends: Vec::new() };
        let drawn = match m.fill(record)? {
            Ok(painted) => painted,
            Err(why) => {
                check.undrawn = Some(why);
                out.push(check);
                continue;
            }
        };
        if let Setting::Drawn(gradient) = psd_gradient::gradient_of(&settings, header.mode)? {
            let bounds = m.gradient_box(record, &gradient, m.layer_box(record)?)?;
            let (start, end) = gradient.ends(bounds);
            let magnitude = psd_gradient::operands(bounds, start, end);
            check.ends = start
                .iter()
                .chain(&end)
                .map(|&v| {
                    let apart = (v - v.round()).abs();
                    [v, apart, apart / (f64::EPSILON * magnitude)]
                })
                .collect();
        }
        if !record.has_pixels() {
            out.push(check);
            continue;
        }
        let stored = m.object(record)?;
        let rect = record.rect;
        let mut region: Vec<bool> = (0..width * height)
            .map(|i| {
                let (x, y) = ((i % width) as i32, (i / width) as i32);
                x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
            })
            .collect();
        if let Some(data) = record.vector_mask() {
            let coverage = psd_vector::read(data)?.coverage_rows(width, height, 0, height);
            for (r, c) in region.iter_mut().zip(&coverage) {
                *r = *r && *c >= 1.0 - 1e-6;
            }
            if let Some(stroke) = record.stroke()?
                && stroke.flag("strokeEnabled") == Some(true)
            {
                let wide = match stroke.get("strokeStyleLineWidth") {
                    Some(Value::Unit(_, w)) | Some(Value::Double(w)) => *w,
                    _ => 0.0,
                };
                let side = match stroke.get("strokeStyleLineAlignment") {
                    Some(Value::Enum(_, side)) => side.clone(),
                    _ => String::new(),
                };
                let band = if side == "strokeStyleAlignInside" { wide.ceil() as usize + 1 } else if side == "strokeStyleAlignCenter" { (wide / 2.0).ceil() as usize + 1 } else { 1 };
                region = worn(&region, width, height, band);
            }
        }
        let (mut color_apart, mut shape_apart) = (Vec::new(), Vec::new());
        for (i, &inside) in region.iter().enumerate() {
            if !inside {
                continue;
            }
            check.compared += 1;
            for c in 0..channels {
                color_apart.push((f64::from(drawn.color[c][i]) - f64::from(stored.color[c][i])).abs() * 255.0);
            }
            shape_apart.push((f64::from(drawn.shape[i]) - f64::from(stored.shape[i])).abs() * 255.0);
        }
        check.color = spread(&color_apart);
        check.shape = spread(&shape_apart);
        out.push(check);
    }
    Ok(out)
}
