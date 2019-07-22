use clap::{App, Arg, ArgMatches, SubCommand};
use humansize::{file_size_opts::BINARY, FileSize};
use rc_zip::prelude::*;
use std::fmt;
use std::fs::File;
use std::io::Read;

struct Optional<T>(Option<T>);

impl<T> fmt::Display for Optional<T>
where
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(x) = self.0.as_ref() {
            write!(f, "{}", x)
        } else {
            write!(f, "∅")
        }
    }
}

impl<T> fmt::Debug for Optional<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(x) = self.0.as_ref() {
            write!(f, "{:?}", x)
        } else {
            write!(f, "∅")
        }
    }
}

fn main() {
    #[cfg(feature = "color-backtrace")]
    color_backtrace::install();
    #[cfg(feature = "env_logger")]
    env_logger::init();

    let matches = App::new("rc-zip sample")
        .subcommand(
            SubCommand::with_name("info")
                .about("Show information about a ZIP file")
                .arg(
                    Arg::with_name("file")
                        .help("ZIP file to analyze")
                        .required(true)
                        .index(1),
                ),
        )
        .subcommand(
            SubCommand::with_name("list")
                .about("List files contained in a ZIP file")
                .arg(
                    Arg::with_name("file")
                        .help("ZIP file to list")
                        .required(true)
                        .index(1),
                )
                .arg(
                    Arg::with_name("verbose")
                        .help("Show verbose information for each file")
                        .long("--verbose")
                        .short("v"),
                ),
        )
        .subcommand(
            SubCommand::with_name("extract")
                .about("Extract files contained in a ZIP archive")
                .arg(
                    Arg::with_name("file")
                        .help("ZIP file to extract")
                        .required(true)
                        .index(1),
                ),
        )
        .subcommand(
            SubCommand::with_name("compress")
                .about("Add files to a ZIP archive")
                .arg(
                    Arg::with_name("files")
                        .help("Files to add to the archive")
                        .required(true)
                        .multiple(true)
                        .index(1),
                )
                .arg(
                    Arg::with_name("output")
                        .help("Path of the zip file to crate")
                        .required(true)
                        .long("--output")
                        .short("-o"),
                ),
        )
        .get_matches();

    do_main(matches).unwrap();
}

fn do_main(matches: ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
    fn info(archive: &rc_zip::Archive) {
        if let Some(comment) = archive.comment() {
            println!("Comment:\n{}", comment);
        }

        use std::collections::HashSet;
        let mut creator_versions = HashSet::<rc_zip::Version>::new();
        let mut reader_versions = HashSet::<rc_zip::Version>::new();
        let mut methods = HashSet::<rc_zip::Method>::new();
        let mut compressed_size: u64 = 0;
        let mut uncompressed_size: u64 = 0;
        let mut num_dirs = 0;
        let mut num_symlinks = 0;
        let mut num_files = 0;

        for entry in archive.entries() {
            creator_versions.insert(entry.creator_version);
            reader_versions.insert(entry.reader_version);
            match entry.contents() {
                rc_zip::EntryContents::Symlink(_) => {
                    num_symlinks += 1;
                }
                rc_zip::EntryContents::Directory(_) => {
                    num_dirs += 1;
                }
                rc_zip::EntryContents::File(f) => {
                    methods.insert(entry.method());
                    num_files += 1;
                    compressed_size += f.entry.compressed_size;
                    uncompressed_size += f.entry.uncompressed_size;
                }
            }
        }
        println!(
            "Version made by: {:?}, required: {:?}",
            creator_versions, reader_versions
        );
        println!("Encoding: {}, Methods: {:?}", archive.encoding(), methods);
        println!(
            "{} ({:.2}% compression) ({} files, {} dirs, {} symlinks)",
            uncompressed_size.file_size(BINARY).unwrap(),
            compressed_size as f64 / uncompressed_size as f64 * 100.0,
            num_files,
            num_dirs,
            num_symlinks,
        );
    }

    match matches.subcommand() {
        ("info", Some(matches)) => {
            let reader = File::open(matches.value_of("file").unwrap())?.read_zip()?;
            info(&reader);
        }
        ("list", Some(matches)) => {
            let file = File::open(matches.value_of("file").unwrap())?;
            let reader = file.read_zip()?;
            let verbose = matches.is_present("verbose");

            use std::io::Write;
            use tabwriter::TabWriter;

            let mut stdout = std::io::stdout();
            let mut tw = TabWriter::new(&mut stdout);
            write!(&mut tw, "Mode\tName\tSize")?;
            if verbose {
                write!(&mut tw, "\tModified\tUID\tGID")?;
            }
            writeln!(&mut tw)?;

            for entry in reader.entries() {
                write!(
                    &mut tw,
                    "{mode}\t{name}\t{size}",
                    mode = entry.mode,
                    name = entry.name().truncate_path(55),
                    size = entry.uncompressed_size.file_size(BINARY).unwrap(),
                )?;
                if verbose {
                    write!(
                        &mut tw,
                        "\t{modified}\t{uid}\t{gid}",
                        modified = entry.modified(),
                        uid = Optional(entry.uid),
                        gid = Optional(entry.gid),
                    )?;

                    match entry.contents() {
                        rc_zip::EntryContents::Symlink(sl) => {
                            let mut target = String::new();
                            rc_zip::EntryReader::new(sl.entry, |offset| {
                                positioned_io::Cursor::new_pos(&file, dbg!(offset))
                            })
                            .read_to_string(&mut target)
                            .unwrap();
                            write!(&mut tw, "\t{target}", target = target)?;
                        }
                        _ => {}
                    }
                }
                writeln!(&mut tw)?;
            }
            tw.flush()?;
        }
        ("extract", Some(matches)) => {
            let file = File::open(matches.value_of("file").unwrap())?;
            let reader = file.read_zip()?;
            info(&reader);

            for entry in reader.entries() {
                println!("Extracting {}", entry.name());
                let mut contents = Vec::<u8>::new();
                entry
                    .reader(|offset| positioned_io::Cursor::new_pos(&file, offset))
                    .read_to_end(&mut contents)?;

                if let Ok(s) = std::str::from_utf8(&contents[..]) {
                    println!("contents = {:?}", s);
                } else {
                    println!("contents = {:?}", contents);
                }
            }
        }
        ("compress", Some(matches)) => {
            let files = matches.values_of("files").unwrap();
            let output = matches.value_of("output").unwrap();
            println!("Should add {:?} to archive {:?}", files, output);
            unimplemented!();
        }
        _ => {
            println!("{}", matches.usage());
            std::process::exit(1);
        }
    }

    Ok(())
}

trait Truncate {
    fn truncate_path(&self, limit: usize) -> String;
}

impl Truncate for &str {
    fn truncate_path(&self, limit: usize) -> String {
        let mut name_tokens: Vec<&str> = Vec::new();
        let mut rest_tokens: std::collections::VecDeque<&str> = self.split('/').collect();
        loop {
            let len_separators = name_tokens.len() + rest_tokens.len() - 1;
            let len_strings = name_tokens.iter().map(|x| x.len()).sum::<usize>()
                + rest_tokens.iter().map(|x| x.len()).sum::<usize>();
            if len_separators + len_strings < limit {
                name_tokens.extend(rest_tokens.into_iter());
                break name_tokens.join("/");
            }
            if rest_tokens.len() == 0 {
                name_tokens.extend(rest_tokens.into_iter());
                let name = name_tokens.join("/");
                break name.chars().take(limit - 3).collect::<String>() + "...";
            }
            let token = rest_tokens.pop_front().unwrap();
            match token.char_indices().skip(1).next() {
                Some((i, _)) => name_tokens.push(&token[..i]),
                None => name_tokens.push(token),
            }
        }
    }
}
