//! Command line front end for the Open CE.

mod collision;
mod debug;
mod map;
mod scene;
mod settings;
mod terrain;
mod world;

use std::collections::BTreeMap;
use std::env;
use std::error::Error as DynError;
use std::fs;
use std::path::PathBuf;

use oce_bsa::Archive;
use oce_core::{Error, FormId};
use oce_data::{AssetLocation, AssetResolver, DataDir, Game, LoadOrder};
use oce_nif::{classify, BlockRole, Nif};
use oce_plugin::{strings::StringsFile, Plugin, WalkEntry};

type CliResult = Result<(), Box<dyn DynError>>;

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "help" || arg == "--help" || arg == "-h")
    {
        print_help();
        return;
    }
    if args.is_empty() {
        args.push("run".to_string());
    }
    if let Err(error) = run(&args) {
        eprintln!("oce: {error}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> CliResult {
    match args[0].as_str() {
        "run" => run_game(&args[1..]),
        "doctor" => doctor(args.get(1).map(String::as_str)),
        "plugin" => plugin_command(&args[1..]),
        "bsa" => bsa_command(&args[1..]),
        "asset" => asset_command(&args[1..]),
        "nif" => nif_command(&args[1..]),
        "dds" => dds_command(&args[1..]),
        "strings" => strings_command(&args[1..]),
        other => Err(Error::invalid(format!("unknown command '{other}' (try 'oce help')")).into()),
    }
}

fn plugin_command(args: &[String]) -> CliResult {
    match args.first().map(String::as_str) {
        Some("info") => plugin_info(arg(args, 1, "plugin path")?),
        Some("records") => plugin_records(&args[1..]),
        Some("record") => plugin_record(&args[1..]),
        Some(other) => Err(Error::invalid(format!("unknown plugin command '{other}'")).into()),
        None => Err(Error::invalid("usage: oce plugin <info|records|record> ...").into()),
    }
}

fn bsa_command(args: &[String]) -> CliResult {
    match args.first().map(String::as_str) {
        Some("info") => bsa_info(arg(args, 1, "archive path")?),
        Some("list") => bsa_list(&args[1..]),
        Some("extract") => bsa_extract(&args[1..]),
        Some(other) => Err(Error::invalid(format!("unknown bsa command '{other}'")).into()),
        None => Err(Error::invalid("usage: oce bsa <info|list|extract> ...").into()),
    }
}

fn asset_command(args: &[String]) -> CliResult {
    match args.first().map(String::as_str) {
        Some("find") => asset_find(&args[1..]),
        Some(other) => Err(Error::invalid(format!("unknown asset command '{other}'")).into()),
        None => Err(Error::invalid("usage: oce asset find <game> <asset>").into()),
    }
}

fn nif_command(args: &[String]) -> CliResult {
    match args.first().map(String::as_str) {
        Some("inspect") => nif_inspect(arg(args, 1, "nif file")?),
        Some("skin") => nif_skin(arg(args, 1, "nif file")?),
        Some("scan") => nif_scan(&args[1..]),
        Some(other) => Err(Error::invalid(format!("unknown nif command '{other}'")).into()),
        None => Err(Error::invalid("usage: oce nif <inspect|skin|scan> ...").into()),
    }
}

fn dds_command(args: &[String]) -> CliResult {
    match args.first().map(String::as_str) {
        Some("inspect") => dds_inspect(arg(args, 1, "dds file")?),
        Some(other) => Err(Error::invalid(format!("unknown dds command '{other}'")).into()),
        None => Err(Error::invalid("usage: oce dds inspect <dds-file>").into()),
    }
}

fn dds_inspect(path: &str) -> CliResult {
    let bytes = fs::read(path)?;
    let info = oce_dds::parse(&bytes)?;
    println!("dds:       {path}");
    println!("format:    {}", info.format);
    println!("size:      {}x{}", info.width, info.height);
    println!("mips:      {}", info.mip_count);
    println!("data:      offset {}", info.data_offset);
    if info.format.is_bc5() || info.format.is_bc7() {
        let blocks_x = info.width.div_ceil(4) as usize;
        let blocks_y = info.height.div_ceil(4) as usize;
        let bytes = blocks_x * blocks_y * 16;
        println!("compressed: {bytes} bytes, top mip");
    }
    Ok(())
}

fn nif_inspect(path: &str) -> CliResult {
    let bytes = fs::read(path)?;
    let nif = Nif::parse(&bytes)?;
    println!("nif:       {path}");
    println!(
        "version:   0x{:08X} (user {})",
        nif.version(),
        nif.user_version()
    );
    println!("blocks:    {}", nif.blocks().len());
    println!("strings:   {}", nif.strings().len());
    println!("types:");
    for (name, count) in nif.histogram() {
        println!("  {count:5}  {name}");
    }
    println!("\ntree:");
    let mut visited = vec![false; nif.blocks().len()];
    print_nif_node(&nif, 0, 0, &mut visited);
    Ok(())
}

fn nif_skin(path: &str) -> CliResult {
    let bytes = fs::read(path)?;
    let nif = Nif::parse(&bytes)?;
    let summaries = nif.skin_summaries()?;
    if summaries.is_empty() {
        println!("no skinned shapes");
        return Ok(());
    }
    for summary in summaries {
        println!(
            "skin: shape={} bones={} dismember={} has_weights={} resolved_bones={}",
            summary.shape,
            summary.bones,
            summary.dismember,
            summary.has_vertex_weights,
            summary.resolved_bones
        );
        println!(
            "descriptor=0x{:016X} stride={} vertices={} weights={}",
            summary.descriptor, summary.vertex_size, summary.vertices, summary.weights_per_vertex
        );
        println!("partitions={}", summary.partitions.len());
        for (index, partition) in summary.partitions.iter().enumerate() {
            println!(
                "  partition={index} vertices={} triangles={}",
                partition.vertices, partition.triangles
            );
        }
        println!("triangles={}", summary.triangles);
        println!("bind_deviation={:.6}", summary.bind_deviation);
    }
    Ok(())
}

fn print_nif_node(nif: &Nif, index: usize, depth: usize, visited: &mut [bool]) {
    if index >= nif.blocks().len() || visited[index] {
        return;
    }
    visited[index] = true;
    let Some(block) = nif.blocks().get(index) else {
        return;
    };
    let indent = "  ".repeat(depth);
    match nif.node(index) {
        Ok(Some(node)) => {
            println!("{indent}{index:4}  {}", block.type_name);
            for child in node.children {
                if child < 0 {
                    println!("{indent}        -1");
                } else {
                    print_nif_node(nif, child as usize, depth + 1, visited);
                }
            }
        }
        Ok(None) if block.type_name == "BSTriShape" => {
            let texture = nif.texture_for_shape(index).unwrap_or_default();
            let alpha = nif.alpha_for_shape(index);
            println!(
                "{indent}{index:4}  {}  alpha=0x{alpha:04X}  texture={texture}",
                block.type_name
            );
        }
        Ok(None) => println!("{indent}{index:4}  {}", block.type_name),
        Err(error) => println!("{indent}{index:4}  {}  <{error}>", block.type_name),
    }
}

fn nif_scan(args: &[String]) -> CliResult {
    let mut game_path: Option<PathBuf> = None;
    let mut filter = None;
    let mut limit = usize::MAX;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--filter" => {
                filter = Some(
                    args.get(index + 1)
                        .ok_or_else(|| Error::invalid("missing value for --filter"))?
                        .to_ascii_lowercase(),
                );
                index += 1;
            }
            "--limit" => {
                limit = args
                    .get(index + 1)
                    .ok_or_else(|| Error::invalid("missing value for --limit"))?
                    .parse()
                    .map_err(|_| Error::invalid("invalid --limit value"))?;
                index += 1;
            }
            value if value.starts_with("--") => {
                return Err(Error::invalid(format!("unknown nif scan option '{value}'")).into())
            }
            value if game_path.is_none() => game_path = Some(PathBuf::from(value)),
            _ => {
                return Err(Error::invalid(
                    "usage: oce nif scan [game] [--limit N] [--filter TEXT]",
                )
                .into())
            }
        }
        index += 1;
    }

    let mut game = match game_path {
        Some(path) => Game::open(path)?,
        None => Game::detect()?,
    };
    game.load_archives()?;

    let mut scanned = 0usize;
    let mut failed = 0usize;
    let mut histogram = BTreeMap::new();
    let mut unknown = BTreeMap::new();

    'archives: for archive in game.assets.archives() {
        for (file_index, file) in archive.files().iter().enumerate() {
            let path = file.path.to_ascii_lowercase();
            if !path.ends_with(".nif") {
                continue;
            }
            if let Some(filter) = &filter {
                if !path.contains(filter) {
                    continue;
                }
            }
            if scanned >= limit {
                break 'archives;
            }
            scanned += 1;
            match archive
                .read_file(file_index)
                .and_then(|bytes| Nif::parse(&bytes))
            {
                Ok(nif) => {
                    for (name, count) in nif.histogram() {
                        *histogram.entry(name.clone()).or_insert(0) += count;
                        if classify(&name) == BlockRole::Unknown {
                            *unknown.entry(name).or_insert(0) += count;
                        }
                    }
                }
                Err(error) => {
                    failed += 1;
                    eprintln!("failed: {}: {error}", file.path);
                }
            }
        }
    }

    println!("nif scan: {scanned} files, {failed} failed");
    println!("block types:");
    for (name, count) in &histogram {
        println!("  {count:8}  {name}");
    }
    if unknown.is_empty() {
        println!("unknown block types: none");
    } else {
        println!("unknown block types:");
        for (name, count) in &unknown {
            println!("  {count:8}  {name}");
        }
    }
    Ok(())
}

fn strings_command(args: &[String]) -> CliResult {
    let path = arg(args, 0, "strings file")?;
    let table = StringsFile::open(path)?;
    let limit = flag_value(args, "--limit")
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(usize::MAX);
    for (id, value) in table.entries().take(limit) {
        println!("{id:08X}\t{value}");
    }
    Ok(())
}

fn run_game(args: &[String]) -> CliResult {
    let mut game_path: Option<PathBuf> = None;
    let mut headless = false;
    let mut debug = false;
    let mut map_mode = false;
    let mut test_cell = false;
    let mut frames: Option<usize> = None;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--headless" => headless = true,
            "--debug" => debug = true,
            "--map" => map_mode = true,
            "--test-cell" => test_cell = true,
            "--frames" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| Error::invalid("missing value for --frames"))?;
                frames = Some(
                    value
                        .parse()
                        .map_err(|_| Error::invalid(format!("invalid frame count '{value}'")))?,
                );
                index += 1;
            }
            "--game" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| Error::invalid("missing value for --game"))?;
                game_path = Some(PathBuf::from(value));
                index += 1;
            }
            value if value.starts_with("--") => {
                return Err(Error::invalid(format!("unknown run option '{value}'")).into())
            }
            value if game_path.is_none() => game_path = Some(PathBuf::from(value)),
            _ => return Err(Error::invalid(
                "usage: oce run [game] [--headless] [--debug] [--map] [--test-cell] [--frames N]",
            )
            .into()),
        }
        index += 1;
    }

    if map_mode && test_cell {
        return Err(Error::invalid("--map and --test-cell cannot be combined").into());
    }
    if map_mode && debug {
        return Err(Error::invalid("--debug requires the 3D scene, not --map").into());
    }

    let mut game = match game_path {
        Some(path) => Game::open(path)?,
        None => Game::detect()?,
    };
    println!("game:      {}", game.data.root().display());
    println!("data:      {}", game.data.path().display());
    println!("plugins:   {}", game.data.plugins()?.len());
    println!("archives:  {}", game.data.archives()?.len());
    println!(
        "order:     {} enabled, {} missing",
        game.load_order.enabled().count(),
        game.load_order
            .entries
            .iter()
            .filter(|entry| entry.path.is_none())
            .count()
    );

    let report = game.load_archives()?;
    println!(
        "loaded:    {} archives, {} failed",
        report.loaded,
        report.failed.len()
    );
    for (path, error) in report.failed.iter().take(5) {
        eprintln!("  {}: {error}", path.display());
    }

    if test_cell {
        return run_test_cell(&game, headless, debug, frames);
    }

    let plugin = game
        .plugin("Skyrim.esm")?
        .ok_or_else(|| Error::invalid("Skyrim.esm not found"))?;
    let cells = map::build_cells(&plugin)?;
    let refs: u64 = cells.iter().map(|cell| u64::from(cell.refs)).sum();
    let max_refs = cells.iter().map(|cell| cell.refs).max().unwrap_or(0);
    println!(
        "map:       {} Tamriel exterior cells, {} references, max {}/cell",
        cells.len(),
        refs,
        max_refs
    );
    if let Some(cell) = cells.iter().max_by_key(|cell| cell.refs) {
        println!(
            "densest:   cell ({}, {}) with {} references",
            cell.x, cell.y, cell.refs
        );
    }

    if map_mode {
        if !headless {
            map::run(&cells, frames)?;
        }
        return Ok(());
    }

    let settings = settings::GameSettings::load(&plugin)?;
    let controller = collision::ControllerConfig::from_settings(&settings);
    let terrain = terrain::Terrain::load(&plugin, (0, 0), 1)?;
    let mesh = terrain.mesh();
    let (min_height, max_height) = terrain.height_range();
    println!(
        "terrain:   {} cells, {} vertices, {} triangles, height {:.0}..{:.0}",
        terrain.cell_count(),
        mesh.vertices.len(),
        mesh.indices.len() / 3,
        min_height,
        max_height
    );

    let scene = scene::Scene::load(&game, &plugin)?;
    println!(
        "scene:     {} objects, {} textures, {} models loaded, {} failed, {} textures failed",
        scene.objects.len(),
        scene.textures.len(),
        scene.loaded_models,
        scene.failed_models,
        scene.failed_textures
    );
    print_texture_formats(&scene);

    if let Some((min, max)) = scene.bounds() {
        println!(
            "bounds:    x {:.0}..{:.0}, y {:.0}..{:.0}, z {:.0}..{:.0}",
            min.x, max.x, min.y, max.y, min.z, max.z
        );
    }

    let collision = collision::CollisionWorld::build(&scene);
    println!(
        "collision: {} triangles, {} objects",
        collision.triangle_count(),
        collision.object_count()
    );
    println!(
        "controller: walk {:.0}, run {:.0}, jump {:.0}, gravity {:.0}",
        controller.walk_speed, controller.run_speed, controller.jump_height, controller.gravity
    );

    if headless {
        if debug {
            println!("debug: headless, no movement samples");
        }
        return Ok(());
    }

    world::run(terrain, scene, controller, collision, debug, frames)?;
    Ok(())
}

fn print_texture_formats(scene: &scene::Scene) {
    let counts = scene.texture_format_counts();
    if counts.is_empty() {
        return;
    }
    let text = counts
        .iter()
        .map(|(format, count)| format!("{format}={count}"))
        .collect::<Vec<_>>()
        .join(", ");
    println!("formats:   {text}");
}

fn run_test_cell(game: &Game, headless: bool, debug: bool, frames: Option<usize>) -> CliResult {
    let terrain = terrain::Terrain::flat();
    let mesh = terrain.mesh();
    println!("cell:      phase2-test");
    println!(
        "terrain:   flat, {} vertices, {} triangles",
        mesh.vertices.len(),
        mesh.indices.len() / 3
    );

    let scene = scene::Scene::load_test_cell(game)?;
    println!(
        "scene:     {} objects, {} textures, {} models loaded, {} failed, {} textures failed",
        scene.objects.len(),
        scene.textures.len(),
        scene.loaded_models,
        scene.failed_models,
        scene.failed_textures
    );
    print_texture_formats(&scene);
    if let Some((min, max)) = scene.bounds() {
        println!(
            "bounds:    x {:.0}..{:.0}, y {:.0}..{:.0}, z {:.0}..{:.0}",
            min.x, max.x, min.y, max.y, min.z, max.z
        );
    }

    let collision = collision::CollisionWorld::build(&scene);
    println!(
        "collision: {} triangles, {} objects",
        collision.triangle_count(),
        collision.object_count()
    );
    let controller = collision::ControllerConfig::default();
    println!(
        "controller: walk {:.0}, run {:.0}, jump {:.0}, gravity {:.0}",
        controller.walk_speed, controller.run_speed, controller.jump_height, controller.gravity
    );

    if headless {
        if debug {
            println!("debug: headless, no movement samples");
        }
        return Ok(());
    }

    world::run(terrain, scene, controller, collision, debug, frames)?;
    Ok(())
}

fn doctor(game: Option<&str>) -> CliResult {
    let data = match game {
        Some(path) => DataDir::open(path)?,
        None => DataDir::detect()?,
    };
    println!("game root: {}", data.root().display());
    println!("data dir:  {}", data.path().display());

    let plugins = data.plugins()?;
    let archives = data.archives()?;
    println!("plugins:   {}", plugins.len());
    println!("archives:  {}", archives.len());

    println!("core masters:");
    for path in data.core_masters() {
        let header = oce_plugin::read_header(&path)?;
        let size = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        println!(
            "  {:<16} {:>12} bytes  flags=0x{:08X}  masters={}  hed={:.4}",
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("?"),
            size,
            header.flags,
            header.masters.len(),
            header.hed_version.unwrap_or(0.0)
        );
    }

    if let Some(path) = data.plugins_txt() {
        let order = LoadOrder::from_plugins_txt(&data, &path)?;
        let enabled = order.enabled().count();
        let missing = order
            .entries
            .iter()
            .filter(|entry| entry.path.is_none())
            .count();
        println!(
            "load order: {} ({} entries, {} enabled, {} missing)",
            path.display(),
            order.entries.len(),
            enabled,
            missing
        );
    } else {
        println!("load order: no plugins.txt found");
    }
    Ok(())
}

fn plugin_info(path: &str) -> CliResult {
    let plugin = Plugin::open(path)?;
    let header = plugin.header();
    println!("plugin:    {}", plugin.path().display());
    println!("size:      {} bytes", plugin.data().len());
    println!("kind:      {}", plugin.kind().as_str());
    println!(
        "flags:     0x{:08X}{}",
        header.flags,
        plugin_flags_text(header.flags)
    );
    if let Some(version) = header.hed_version {
        println!("hedr:      {version:.4}");
    }
    println!("masters:   {}", header.masters.len());
    for (index, master) in header.masters.iter().enumerate() {
        println!("  [{index:02X}] {master}");
    }
    if let Some(author) = &header.author {
        println!("author:    {author}");
    }
    if let Some(description) = &header.description {
        println!("summary:   {description}");
    }
    println!("overrides: {}", header.overrides.len());

    let stats = plugin.walk(|_| Ok(()))?;
    let total = stats.records + stats.groups;
    match header.num_records {
        Some(declared) => println!(
            "records:   {} records, {} groups, {} total (HEDR {})",
            stats.records, stats.groups, total, declared
        ),
        None => println!(
            "records:   {} records, {} groups, {} total",
            stats.records, stats.groups, total
        ),
    }
    println!(
        "compressed: {} records, localized: {} records",
        stats.compressed_records, stats.localized_records
    );

    let mut types: Vec<_> = stats.record_types.into_iter().collect();
    types.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    println!("top record types:");
    for (signature, count) in types.into_iter().take(12) {
        println!("  {:<4} {}", String::from_utf8_lossy(&signature), count);
    }
    Ok(())
}

fn plugin_records(args: &[String]) -> CliResult {
    let path = arg(args, 0, "plugin path")?;
    let filter = flag_value(args, "--type")
        .map(signature_from_arg)
        .transpose()?;
    let limit = flag_value(args, "--limit")
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(usize::MAX);
    let plugin = Plugin::open(path)?;
    let mut printed = 0usize;
    plugin.walk(|entry| {
        if let WalkEntry::Record {
            header,
            body,
            offset,
            depth,
        } = entry
        {
            if printed >= limit {
                return Ok(());
            }
            if let Some(filter) = filter {
                if header.signature != filter {
                    return Ok(());
                }
            }
            println!(
                "{:08X} {:<4} depth={} size={} offset=0x{:X} flags=0x{:08X}{}",
                header.form_id.raw(),
                header.type_name(),
                depth,
                header.data_size,
                offset,
                header.flags,
                if header.is_compressed() {
                    " compressed"
                } else {
                    ""
                }
            );
            let _ = body;
            printed += 1;
        }
        Ok(())
    })?;
    Ok(())
}

fn plugin_record(args: &[String]) -> CliResult {
    let path = arg(args, 0, "plugin path")?;
    let raw = arg(args, 1, "form id")?;
    let form_id = parse_form_id(raw)?;
    let plugin = Plugin::open(path)?;
    let record = plugin
        .find_record(form_id)?
        .ok_or_else(|| Error::invalid(format!("form {form_id} not found")))?;
    println!(
        "record: {} at 0x{:X}, size {}, flags 0x{:08X}{}",
        record.header.type_name(),
        record.offset,
        record.header.data_size,
        record.header.flags,
        plugin_flags_text(record.header.flags)
    );
    println!(
        "form:   {:08X}  version {}  vci 0x{:08X}/0x{:04X}",
        record.header.form_id.raw(),
        record.header.form_version,
        record.header.version_control_info,
        record.header.version_control_info2
    );
    let decoded = record.decode()?;
    let subrecords = decoded.subrecords()?;
    println!("subrecords: {}", subrecords.len());
    for subrecord in subrecords {
        let preview = preview_bytes(subrecord.data, 32);
        println!(
            "  {:<4} {:>6} bytes  {}",
            subrecord.type_name(),
            subrecord.data.len(),
            preview
        );
    }
    Ok(())
}

fn bsa_info(path: &str) -> CliResult {
    let archive = Archive::open(path)?;
    let header = archive.header();
    println!("archive:   {}", archive.path().display());
    println!("version:   {}", header.version);
    println!(
        "flags:     0x{:08X}{}",
        header.archive_flags,
        archive_flags_text(header.archive_flags)
    );
    println!(
        "file flags: 0x{:08X}{}",
        header.file_flags,
        file_flags_text(header.file_flags)
    );
    println!(
        "folders:   {} ({} name bytes)",
        header.folder_count, header.total_folder_name_length
    );
    println!(
        "files:     {} ({} name bytes)",
        header.file_count, header.total_file_name_length
    );
    if let Some(data_start) = archive.files().iter().map(|file| file.offset).min() {
        println!("data start: 0x{data_start:X}");
    }
    if header.compressed() {
        let compressed = archive
            .files()
            .iter()
            .filter(|file| file.is_compressed(header))
            .count();
        println!(
            "compression: {compressed} compressed, {} stored",
            archive.file_count() - compressed
        );
    }
    if header.embedded_file_names() {
        println!("embedded file names: yes");
    }
    Ok(())
}

fn bsa_list(args: &[String]) -> CliResult {
    let path = arg(args, 0, "archive path")?;
    let filter = flag_value(args, "--filter").map(|value| value.to_ascii_lowercase());
    let limit = flag_value(args, "--limit")
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(usize::MAX);
    let archive = Archive::open(path)?;
    let mut printed = 0usize;
    for file in archive.files() {
        if let Some(filter) = &filter {
            if !file.path.to_ascii_lowercase().contains(filter) {
                continue;
            }
        }
        println!("{:>10}  {}", file.stored_size(), file.path);
        printed += 1;
        if printed >= limit {
            break;
        }
    }
    Ok(())
}

fn bsa_extract(args: &[String]) -> CliResult {
    let path = arg(args, 0, "archive path")?;
    let asset = arg(args, 1, "asset path")?;
    let output = PathBuf::from(
        flag_value(args, "--out").ok_or_else(|| Error::invalid("missing --out <file>"))?,
    );
    let archive = Archive::open(path)?;
    let index = archive
        .find(asset)
        .ok_or_else(|| Error::invalid(format!("{asset} not found in {path}")))?;
    let bytes = archive.read_file(index)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(&output, &bytes)?;
    println!(
        "extracted {} bytes from {} to {}",
        bytes.len(),
        archive.files()[index].path,
        output.display()
    );
    Ok(())
}

fn asset_find(args: &[String]) -> CliResult {
    let game = arg(args, 0, "game directory")?;
    let asset = arg(args, 1, "asset path")?;
    let data = DataDir::open(game)?;
    let mut resolver = AssetResolver::new(data);
    let report = resolver.load_all_archives()?;
    if !report.failed.is_empty() {
        eprintln!(
            "warning: {} archive(s) could not be opened",
            report.failed.len()
        );
        for (path, error) in &report.failed {
            eprintln!("  {}: {error}", path.display());
        }
    }
    match resolver.find(asset) {
        Some(AssetLocation::Loose(path)) => println!("loose: {}", path.display()),
        Some(AssetLocation::Archive { archive, path }) => {
            println!("archive: {}", archive.display());
            println!("path:    {path}");
        }
        None => {
            return Err(Error::invalid(format!(
                "{asset} not found in loose files or {} archives",
                report.loaded
            ))
            .into())
        }
    }
    Ok(())
}

fn arg<'a>(args: &'a [String], index: usize, name: &str) -> Result<&'a str, Box<dyn DynError>> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| Error::invalid(format!("missing {name}")).into())
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn parse_form_id(value: &str) -> Result<FormId, Box<dyn DynError>> {
    let value = value.trim();
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    let raw = u32::from_str_radix(value, 16)
        .map_err(|_| Error::invalid(format!("invalid form id '{value}'")))?;
    Ok(FormId(raw))
}

fn signature_from_arg(value: &str) -> Result<[u8; 4], Box<dyn DynError>> {
    let bytes = value.trim().as_bytes();
    if bytes.is_empty() || bytes.len() > 4 {
        return Err(Error::invalid("record type must be 1-4 ASCII characters").into());
    }
    let mut signature = [b' '; 4];
    for (index, byte) in bytes.iter().enumerate() {
        signature[index] = byte.to_ascii_uppercase();
    }
    Ok(signature)
}

fn preview_bytes(bytes: &[u8], limit: usize) -> String {
    let shown = &bytes[..bytes.len().min(limit)];
    let mut text = String::new();
    for byte in shown {
        text.push_str(&format!("{byte:02X}"));
    }
    if bytes.len() > shown.len() {
        text.push_str("...");
    }
    let ascii: String = shown
        .iter()
        .map(|&byte| {
            if (0x20..0x7F).contains(&byte) {
                byte as char
            } else {
                '.'
            }
        })
        .collect();
    format!("{text}  |{ascii}|")
}

fn plugin_flags_text(flags: u32) -> String {
    let mut parts = Vec::new();
    if flags & oce_core::plugin_flags::MASTER != 0 {
        parts.push("master");
    }
    if flags & oce_core::plugin_flags::LOCALIZED != 0 {
        parts.push("localized");
    }
    if flags & oce_core::plugin_flags::LIGHT != 0 {
        parts.push("light");
    }
    if flags & oce_core::record_flags::COMPRESSED != 0 {
        parts.push("compressed");
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

fn archive_flags_text(flags: u32) -> String {
    let mut parts = Vec::new();
    for (bit, name) in [
        (
            oce_core::archive_flags::INCLUDE_DIRECTORY_NAMES,
            "directories",
        ),
        (oce_core::archive_flags::INCLUDE_FILE_NAMES, "file names"),
        (oce_core::archive_flags::COMPRESSED, "compressed"),
        (
            oce_core::archive_flags::RETAIN_DIRECTORY_NAMES,
            "retain directories",
        ),
        (
            oce_core::archive_flags::RETAIN_FILE_NAMES,
            "retain file names",
        ),
        (
            oce_core::archive_flags::RETAIN_FILE_NAME_OFFSETS,
            "retain offsets",
        ),
        (oce_core::archive_flags::XBOX360_ARCHIVE, "xbox360"),
        (
            oce_core::archive_flags::RETAIN_STRINGS_DURING_STARTUP,
            "retain strings",
        ),
        (oce_core::archive_flags::EMBED_FILE_NAMES, "embedded names"),
        (oce_core::archive_flags::XMEM_CODEC, "xmem"),
    ] {
        if flags & bit != 0 {
            parts.push(name);
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

fn file_flags_text(flags: u32) -> String {
    let mut parts = Vec::new();
    for (bit, name) in [
        (oce_core::archive_file_flags::MESHES, "meshes"),
        (oce_core::archive_file_flags::TEXTURES, "textures"),
        (oce_core::archive_file_flags::MENUS, "menus"),
        (oce_core::archive_file_flags::SOUNDS, "sounds"),
        (oce_core::archive_file_flags::VOICES, "voices"),
        (oce_core::archive_file_flags::SHADERS, "shaders"),
        (oce_core::archive_file_flags::TREES, "trees"),
        (oce_core::archive_file_flags::FONTS, "fonts"),
        (oce_core::archive_file_flags::MISCELLANEOUS, "misc"),
    ] {
        if flags & bit != 0 {
            parts.push(name);
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

fn print_help() {
    print!(
        r#"oce - Open CE

usage:
  oce [run] [game] [--map] [--test-cell] [--headless] [--debug] [--frames N]
  oce doctor [game-dir]
  oce plugin info <plugin>
  oce plugin records <plugin> [--type TYPE] [--limit N]
  oce plugin record <plugin> <form-id>
  oce bsa info <archive>
  oce bsa list <archive> [--filter TEXT] [--limit N]
  oce bsa extract <archive> <asset> --out <file>
  oce asset find <game-dir> <asset>
  oce nif inspect <nif-file>
  oce nif skin <nif-file>
  oce nif scan [game-dir] [--limit N] [--filter TEXT]
  oce dds inspect <dds-file>
  oce strings <strings-file> [--limit N]

With no command, oce auto-detects Skyrim SE and opens the first-person
terrain scene. --map opens the 2D cell-density view instead. --test-cell
opens the flat Phase 2 test cell with curated models. --debug prints one
movement and runtime sample line per frame. Controls:
WASD move, Shift run, Space jump, mouse look, Esc quit. The tools read
the game's own files. No game assets are bundled.
"#
    );
}
