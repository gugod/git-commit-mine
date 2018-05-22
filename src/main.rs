extern crate sha1;
#[macro_use]
extern crate structopt;
extern crate num_cpus;

use std::cmp;
use std::cmp::Ordering;
use std::process::Command;
use std::str;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::{Duration, Instant};
use structopt::StructOpt;

#[derive(Clone)]
struct Commit {
    metadata: Vec<u8>,
    message: Vec<u8>,
    prefix: Vec<u8>,
}

impl Commit {
    fn new(metadata: &str, message: &str) -> Commit {
        Commit {
            metadata: string_to_vec(metadata),
            message: string_to_vec(message),
            prefix: Vec::new(),
        }
    }

    fn new_with_prefix(metadata: &str, message: &str, prefix: &str) -> Commit {
        Commit {
            metadata: string_to_vec(metadata),
            message: string_to_vec(message),
            prefix: string_to_vec(prefix),
        }
    }

    fn split_bytes(bytes: Vec<u8>) -> (Vec<u8>, Vec<u8>) {
        let i = Commit::find_splitting_index(&bytes);

        let metadata: Vec<u8> = bytes.iter().take(i).map(|&x| x).collect();
        let message: Vec<u8> = bytes.iter().skip(i + 2).map(|&x| x).collect();

        return (metadata, message);
    }

    fn find_splitting_index(bytes: &Vec<u8>) -> usize {
        match bytes
            .iter()
            .zip(bytes.iter().skip(1))
            .position(|(&a, &b)| a == b'\n' && b == b'\n')
        {
            Some(i) => i,
            None => bytes.len(),
        }
    }

    fn length(&self) -> usize {
        self.metadata.len() + 2 + self.message.len()
    }

    fn prefix_length(&self, nonce: u64) -> usize {
        self.length() + self.prefix.len() + 1 + base_10_length(nonce) + 1
    }

    fn sha1(&self) -> sha1::Digest {
        let mut m = sha1::Sha1::new();

        m.update(format!("commit {}\0", self.length()).as_bytes());
        m.update(self.metadata.as_slice());
        m.update(b"\n\n");
        m.update(self.message.as_slice());

        return m.digest();
    }

    fn annotate(&self, nonce: u64) -> sha1::Digest {
        let mut m = sha1::Sha1::new();

        m.update(format!("commit {}\0", self.prefix_length(nonce)).as_bytes());
        m.update(self.metadata.as_slice());
        m.update(b"\n");
        m.update(self.prefix.as_slice());
        m.update(format!(" {0}\n\n", nonce).as_bytes());
        m.update(self.message.as_slice());

        return m.digest();
    }
}

fn string_to_vec(string: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(string.as_bytes());
    return bytes;
}

fn base_10_length(n: u64) -> usize {
    // TODO: Something better
    format!("{}", n).as_bytes().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_length_1() {
        let c = Commit::new("fooo", "barbar");
        assert_eq!(c.length(), 12);
    }

    #[test]
    fn test_prefix_length_1() {
        let c = Commit::new_with_prefix("fooo", "barbar", "spam");
        assert_eq!(c.length(), 12);
        assert_eq!(c.prefix_length(0), c.length() + 4 + 1 + 1 + 1);
        assert_eq!(c.prefix_length(1), c.length() + 4 + 1 + 1 + 1);
        assert_eq!(c.prefix_length(9), c.length() + 4 + 1 + 1 + 1);
        assert_eq!(c.prefix_length(10), c.length() + 4 + 1 + 2 + 1);
        assert_eq!(c.prefix_length(11), c.length() + 4 + 1 + 2 + 1);
    }

    #[test]
    fn test_sha1_1() {
        let c = Commit::new(
            "tree 3a52ea9c086dae34c11faa2822d59fca1170de79
author Gunnar Þór Magnússon <gunnar.magnusson@booking.com> 1526705189 +0200
committer Gunnar Þór Magnússon <gunnar.magnusson@booking.com> 1526705189 +0200",
            "Calculate length of Commits\n",
        );

        let exp = "dfae4d199157e7f5c6b2f81cddb102215db12fa3";
        assert_eq!(c.sha1().to_string(), exp);
    }

    #[test]
    fn test_annotate_1() {
        let c = Commit::new_with_prefix(
            "tree 4ea62912d025c113066dab31e6135bd76277af91
parent dfae4d199157e7f5c6b2f81cddb102215db12fa3
author Gunnar Þór Magnússon <gunnar.magnusson@booking.com> 1526714241 +0200
committer Gunnar Þór Magnússon <gunnar.magnusson@booking.com> 1526714241 +0200",
            "Calculate sha1 of commits\n",
            "gthm-id",
        );

        let exp = "ac7569d5798d67bad1b80d8aa43245aca8b5fdec";
        assert_eq!(c.annotate(100).to_string(), exp);
    }

    #[test]
    fn test_split_bytes_1() {
        let bs = string_to_vec("asdf\n\nqwer");
        let (got1, got2) = Commit::split_bytes(bs);
        let exp1 = string_to_vec("asdf");
        let exp2 = string_to_vec("qwer");

        assert_eq!(got1, exp1);
        assert_eq!(got2, exp2);
    }

    #[test]
    fn test_split_bytes_2() {
        let bs = string_to_vec("asdf");
        let (got1, got2) = Commit::split_bytes(bs);
        let exp1 = string_to_vec("asdf");
        let exp2: Vec<u8> = Vec::new();

        assert_eq!(got1, exp1);
        assert_eq!(got2, exp2);
    }
}

fn count_zeros(hash: std::string::String) -> usize {
    for (i, c) in hash.chars().enumerate() {
        if c != '0' {
            return i;
        }
    }

    return hash.len();
}

#[derive(StructOpt, Debug)]
#[structopt(name = "git-commit-mine")]
struct Opt {
    #[structopt(short = "t", long = "timeout", default_value = "0")]
    timeout: u64,
    #[structopt(short = "z", long = "zeros", default_value = "6")]
    zeros: usize,
    #[structopt(long = "threads", default_value = "0")]
    threads: usize,
    #[structopt(name = "PREFIX")]
    prefix: String,
}

#[derive(Eq, Copy, Clone)]
struct Nugget {
    nonce: u64,
    zeros: usize,
}

impl Nugget {
    fn new(nonce: u64, zeros: usize) -> Nugget {
        Nugget {
            nonce: nonce,
            zeros: zeros,
        }
    }

    fn string(&self, prefix: &String) -> String {
        format!("{} zeros: '{} {}'", self.zeros, prefix, self.nonce)
    }
}

impl Ord for Nugget {
    fn cmp(&self, other: &Nugget) -> Ordering {
        self.zeros.cmp(&other.zeros)
    }
}

impl PartialOrd for Nugget {
    fn partial_cmp(&self, other: &Nugget) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Nugget {
    fn eq(&self, other: &Nugget) -> bool {
        self.zeros == other.zeros
    }
}

fn main() {
    let opt = Opt::from_args();

    let output = Command::new("git")
        .arg("cat-file")
        .arg("commit")
        .arg("HEAD")
        .output()
        .expect("Failed to execute command");

    let (metadata, message) = Commit::split_bytes(output.stdout);
    let c = Commit {
        metadata: metadata,
        message: message,
        prefix: string_to_vec(&opt.prefix),
    };

    let start = Instant::now();
    let timeout = Duration::new(
        match opt.timeout {
            0 => std::u64::MAX,
            _ => opt.timeout,
        },
        0,
    );

    let threads = match opt.threads {
        0 => num_cpus::get(),
        _ => cmp::min(opt.threads, num_cpus::get()),
    };

    let n = Arc::new(AtomicUsize::new(0));
    let (sender, receiver) = channel();

    for _ in 0..threads {
        let n = Arc::clone(&n);
        let results = Sender::clone(&sender);
        let c = c.clone();

        thread::spawn(move || {
            let mut local_best = Nugget::new(0, 0);
            loop {
                // Ordering::Relaxed seems to be OK here according to:
                // https://doc.rust-lang.org/nomicon/atomics.html
                let m = n.fetch_add(1, AtomicOrdering::Relaxed) as u64;

                let b = Nugget::new(m, count_zeros(c.annotate(m).to_string()));
                if local_best.cmp(&b) == Ordering::Less {
                    local_best = b;
                    results.send(b).unwrap();
                }
            }
        });
    }

    let mut best = Nugget::new(0, 0);
    for b in receiver {
        if best.cmp(&b) == Ordering::Less {
            best = b;
            println!("{}", best.string(&opt.prefix));
        }

        if best.zeros >= opt.zeros || start.elapsed() > timeout {
            break;
        }
    }

    println!("Best result: {}", best.string(&opt.prefix));
}
