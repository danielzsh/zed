use fs::repository::GitRepository;

use anyhow::Result;
use parking_lot::Mutex;
use std::{ops::Range, path::Path, sync::Arc};
use sum_tree::SumTree;
use text::Anchor;

pub use git2 as libgit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlameHunkStatus {
    Added,
    Modified,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameHunk<T> {
    pub buffer_range: Range<T>,
    pub diff_base_byte_range: Range<usize>,
}

impl BlameHunk<u32> {
    pub fn status(&self) -> BlameHunkStatus {
        BlameHunkStatus::Added
    }
}

impl sum_tree::Item for BlameHunk<Anchor> {
    type Summary = BlameHunkSummary;

    fn summary(&self) -> Self::Summary {
        BlameHunkSummary {
            buffer_range: self.buffer_range.clone(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct BlameHunkSummary {
    buffer_range: Range<Anchor>,
}

impl sum_tree::Summary for BlameHunkSummary {
    type Context = text::BufferSnapshot;

    fn add_summary(&mut self, other: &Self, buffer: &Self::Context) {
        self.buffer_range.start = self
            .buffer_range
            .start
            .min(&other.buffer_range.start, buffer);
        self.buffer_range.end = self.buffer_range.end.max(&other.buffer_range.end, buffer);
    }
}

#[derive(Clone)]
pub struct BufferBlame {
    tree: SumTree<BlameHunk<Anchor>>,
}

impl BufferBlame {
    pub fn new() -> BufferBlame {
        BufferBlame {
            tree: SumTree::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tree.is_empty()
    }

    pub fn update(
        &mut self,
        repo: Arc<Mutex<dyn GitRepository>>,
        path: &Path,
        buffer: String,
    ) -> Result<()> {
        let repo = repo.lock();

        let blame = repo.blame_path(path)?;
        let blame_buffer = blame.blame_buffer(buffer.as_bytes())?;

        let lines = buffer.lines().collect::<Vec<&str>>();

        for blame_hunk in blame_buffer.iter() {
            let start_line = blame_hunk.final_start_line();
            let lines_in_hunk = blame_hunk.lines_in_hunk();

            if dbg!(dbg!(lines_in_hunk) == usize::MAX) {
                continue;
            }

            dbg!(start_line, lines_in_hunk);
            // let end_line = if lines_in_hunk == 0 {
            //     start_line - 1
            // } else {
            //     start_line + lines_in_hunk - 1
            // };
            // let hunk_lines = &lines[start_line - 1..end_line];

            // let commit_id = blame_hunk.final_commit_id();
            // let uncommitted = commit_id == git2::Oid::zero();
            // for line in hunk_lines {
            //     if uncommitted {
            //         println!(
            //             "{} - NOT COMMITTED - {}",
            //             blame_hunk.final_commit_id(),
            //             line
            //         );
            //     } else {
            //         println!(
            //             "{} - {:?} - {}",
            //             blame_hunk.final_commit_id(),
            //             blame_hunk.final_signature().name(),
            //             line
            //         );
            //     }
            // }
        }
        Ok(())
    }
}
