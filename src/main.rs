use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use log::info;
use rayon::prelude::*;
use sourmash::signature::Signature;
use sourmash::sketch::minhash::{
    max_hash_for_scaled, HashFunctions, KmerMinHash, KmerMinHashBTree,
};
use sourmash::sketch::Sketch;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
struct Cli {
    /// Query to be subtracted
    #[structopt(parse(from_os_str))]
    query: PathBuf,

    /// List of signatures to remove the query from
    #[structopt(parse(from_os_str))]
    siglist: PathBuf,

    /// ksize
    #[structopt(short = "k", long = "ksize", default_value = "31")]
    ksize: u8,

    /// scaled
    #[structopt(short = "s", long = "scaled", default_value = "10")]
    scaled: usize,

    /// The path for output
    #[structopt(parse(from_os_str), short = "o", long = "output")]
    output: Option<PathBuf>,
}

fn subtract<P: AsRef<Path>>(
    query: P,
    siglist: P,
    ksize: u8,
    scaled: usize,
    output: Option<P>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Loading queries");

    let max_hash = max_hash_for_scaled(scaled as u64);
    let template_mh = KmerMinHash::builder()
        .num(0u32)
        .ksize(ksize as u32)
        .hash_function(HashFunctions::murmur64_protein)
        .max_hash(max_hash)
        .build();
    let template = Sketch::MinHash(template_mh);

    let query_sig = Signature::from_path(query).unwrap();
    let mut query: Option<KmerMinHashBTree> = None;
    for sig in &query_sig {
        if let Some(sketch) = sig.select_sketch(&template) {
            if let Sketch::MinHash(mh) = sketch {
                query = Some(mh.clone().into());
            }
        }
    }
    let query = query.unwrap();
    info!("Loaded query signature, k={}", ksize);
    let hashes_to_remove = query.mins();

    info!("Loading siglist");
    let siglist_file = BufReader::new(File::open(siglist)?);
    let search_sigs: Vec<PathBuf> = siglist_file
        .lines()
        .map(|line| {
            let mut path = PathBuf::new();
            path.push(line.unwrap());
            path
        })
        .collect();
    info!("Loaded {} sig paths in siglist", search_sigs.len());

    let mut outdir: PathBuf = if let Some(p) = output {
        p.as_ref().into()
    } else {
        let mut path = PathBuf::new();
        path.push("outputs");
        path
    };
    outdir.push(format!("{}", ksize));
    std::fs::create_dir_all(&outdir)?;

    let processed_sigs = AtomicUsize::new(0);

    search_sigs.par_iter().for_each(|filename| {
        let i = processed_sigs.fetch_add(1, Ordering::SeqCst);
        if i % 1000 == 0 {
            info!("Processed {} sigs", i);
        }

        let mut search_mh = None;
        let mut search_sig = Signature::from_path(&filename)
            .unwrap_or_else(|_| panic!("Error processing {:?}", filename))
            .swap_remove(0);
        if let Some(sketch) = search_sig.select_sketch(&template) {
            if let Sketch::MinHash(mh) = sketch {
                search_mh = Some(mh.clone());
            }
        }
        let mut search_mh = search_mh.unwrap();

        search_mh.remove_many(&hashes_to_remove).unwrap();
        // TODO: save to output dir
        let mut path = outdir.clone();
        path.push(filename.file_name().unwrap());

        let mut out = BufWriter::new(File::create(path).unwrap());
        search_sig.reset_sketches();
        search_sig.push(Sketch::MinHash(search_mh));
        serde_json::to_writer(&mut out, &[search_sig]).unwrap();
    });

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let opts = Cli::from_args();

    subtract(
        opts.query,
        opts.siglist,
        opts.ksize,
        opts.scaled,
        opts.output,
    )?;

    Ok(())
}
