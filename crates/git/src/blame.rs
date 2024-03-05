use fs::repository::GitRepository;

use anyhow::Result;
use libgit::Oid;
use parking_lot::Mutex;
use std::{ops::Range, path::Path, sync::Arc};
use sum_tree::SumTree;
use text::Anchor;

pub use git2 as libgit;

// DiffHunk is the data
// DiffHunk<u32> has a `.status`
// DiffHunk<Anchor> implements `sum_tree::Item`
//   summary is `DiffHunkSummary` with `buffer_range: Range<Anchor>`

// DiffHunkSummary implements `sum_tree::Summary`
// when that gets added via `add_summary` with another summary
// it expands its range.

// BufferDiff, `hunks_in_row_range` takes in a `Range<u32>`
// converts the range to anchors
// then sets `hunks_intersecting_range`, which takes in anchors

// the real magic happens in `hunks_intersecting_range`

// - builds a cursor that only gives the hunks in the tree that are in the range
// - then takes the hunks that cursor returns and turns them into pair of start/end

// - it then calls `buffer.summaries_for_anchors_with_payload` to essentially convert
// the `Anchor`s into `Point`s. The `payload` is the diff information.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameHunk<T> {
    pub buffer_range: Range<T>,
    pub commit: Oid,
}

impl BlameHunk<u32> {}

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
