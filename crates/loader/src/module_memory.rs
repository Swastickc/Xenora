// SPDX-License-Identifier: GPL-3.0-or-later

//! Transactional batch mapping for explicit PE32+ module images.
//!
//! This narrow seam connects a completed [`ModuleLayoutPlan`] to the existing
//! single-image [`ParsedPe::map_into_memory`] primitive. It checks that the
//! plan belongs to the exact ordered input batch before touching memory, maps
//! images in input order at their planned bases and returns logical
//! reservation records for inspection.
//!
//! A complete batch plan has exactly one successful placement for every input
//! image index, with no missing, duplicate or extra indexes. This layer also
//! checks that each planned descriptor matches the parsed image supplied with
//! it and that the half-open placement intervals do not overlap. The overlap
//! and index checks are defensive even though [`ModuleLayoutPlan::build`]
//! currently preserves those invariants: the memory boundary must not depend
//! on mutation order or on a caller composing the wrong plan for a batch.
//!
//! Rollback is deliberately exact and deliberately expensive. For a nonempty
//! batch, after the whole plan validates, the batch clones the entire
//! [`GuestMemory`] once. If image
//! validation or mapping fails later, that clone is assigned back, restoring
//! the prior address space and its metadata rather than merely unmapping the
//! successful images. Each single-image primitive can additionally snapshot
//! the whole address space for its own partial-section rollback. In the worst
//! case this is one whole-memory copy before the batch plus one nested
//! whole-memory copy for every image attempted: time cost scales with the
//! batch length times the committed guest bytes and metadata, while peak host
//! memory includes the live address space, the batch snapshot and one nested
//! image snapshot. This is not the final memory-ownership or efficient
//! reservation design.
//!
//! The returned reservations are logical `[base, end)` spans derived from the
//! planned `SizeOfImage`. They are not ownership tokens. Only headers and
//! sections are committed; holes inside an image span are not inaccessible
//! guard reservations, and this layer does not prevent a later unrelated
//! mapping from using one. It performs no relocation, import or IAT binding,
//! API-set resolution, service registration, exception or TLS initialization,
//! unloading, reference counting or guest execution.

use std::fmt;

use crate::memory::GuestMemory;
use crate::module_layout::{
    AddressInterval, ImageLayoutOutcome, ImagePlacement, ModuleImageDescriptor, ModuleLayoutPlan,
};
use crate::pe::{ParsedPe, PeError};

/// One explicit in-memory module image in batch input order.
///
/// `bytes` must be the complete original PE32+ byte slice represented by `pe`.
/// The batch layer does not parse a second copy. It passes both values to
/// [`ParsedPe::map_into_memory`], which validates header agreement, section
/// extents and mapping arithmetic before insertion.
#[derive(Clone, Copy, Debug)]
pub struct ExplicitModuleImage<'a> {
    pub pe: &'a ParsedPe,
    pub bytes: &'a [u8],
}

/// Logical half-open reservation metadata for one successfully mapped image.
///
/// `end` is the exclusive planned end of the `SizeOfImage` span, not a promise
/// that every byte below it was committed. `entry_point` is absent for a DLL
/// whose entry RVA is zero, matching [`ParsedPe::map_into_memory`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleReservation {
    pub image_index: usize,
    pub base: u64,
    pub end: u64,
    pub used_preferred_base: bool,
    pub entry_point: Option<u64>,
}

/// Errors that prevent a module batch from being mapped as one transaction.
/// Every image-specific failure carries the input `image_index`.
#[derive(Debug, PartialEq, Eq)]
pub enum ModuleMemoryError {
    /// The plan does not have one record for each input image. Because the
    /// exact-one rule is checked before memory mutation, too few and too many
    /// plan records are both batch-shape errors rather than partial mappings.
    PlanInputMismatch {
        plan_images: usize,
        module_images: usize,
    },
    /// A plan record index is outside the input batch or repeats an earlier
    /// record. A repeat necessarily leaves another input index missing.
    InvalidPlanIndex {
        plan_position: usize,
        image_index: usize,
    },
    /// The record for an input image is an explicit planner failure rather
    /// than a successful placement.
    MissingPlacement { image_index: usize },
    /// The descriptor retained by the plan does not match the parsed image
    /// supplied at the same input index.
    DescriptorMismatch { image_index: usize },
    /// Two half-open placement intervals overlap. Adjacent intervals do not.
    OverlappingPlacements {
        first_index: usize,
        first: AddressInterval,
        second_index: usize,
        second: AddressInterval,
        overlap: AddressInterval,
    },
    /// The image's own validation or insertion failed. The batch restored the
    /// exact prior [`GuestMemory`] snapshot before returning this error.
    MappingFailed { image_index: usize, source: PeError },
}

impl fmt::Display for ModuleMemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlanInputMismatch {
                plan_images,
                module_images,
            } => write!(
                f,
                "layout plan has {plan_images} image records for {module_images} module images"
            ),
            Self::InvalidPlanIndex {
                plan_position,
                image_index,
            } => write!(
                f,
                "layout plan record {plan_position} has missing, repeated or out-of-batch image index {image_index}"
            ),
            Self::MissingPlacement { image_index } => {
                write!(f, "image {image_index} has no successful placement")
            }
            Self::DescriptorMismatch { image_index } => {
                write!(f, "layout descriptor does not describe image {image_index}")
            }
            Self::OverlappingPlacements {
                first_index,
                second_index,
                overlap,
                ..
            } => write!(
                f,
                "placements for images {first_index} and {second_index} overlap at [{:#x}, {:#x})",
                overlap.start, overlap.end
            ),
            Self::MappingFailed {
                image_index,
                source,
            } => write!(
                f,
                "image {image_index} failed validation or mapping: {source}"
            ),
        }
    }
}

impl std::error::Error for ModuleMemoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MappingFailed { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Map a complete explicit module batch into `memory` as one transaction.
///
/// Validation of the complete plan finishes before the batch snapshot is made
/// and before any image can mutate memory. Images are then mapped in input
/// order through [`ParsedPe::map_into_memory`]. A failure at any image assigns
/// the pre-batch snapshot back to `memory`, so successful earlier images do
/// not remain mapped.
///
/// On success the reservations are returned in input order. They are logical
/// inspection records only; see the module documentation for the snapshot
/// cost, lack of ownership and absent guard-gap semantics.
pub fn map_module_batch(
    images: &[ExplicitModuleImage<'_>],
    plan: &ModuleLayoutPlan,
    memory: &mut GuestMemory,
) -> Result<Vec<ModuleReservation>, ModuleMemoryError> {
    let placements = validate_complete_plan(images, plan)?;
    if images.is_empty() {
        return Ok(Vec::new());
    }

    let before = memory.clone();
    let mut reservations = Vec::with_capacity(images.len());
    for (image_index, image) in images.iter().enumerate() {
        let placement = placements[image_index];
        match image
            .pe
            .map_into_memory(image.bytes, memory, placement.base)
        {
            Ok(entry_point) => reservations.push(ModuleReservation {
                image_index,
                base: placement.base,
                end: placement.end,
                used_preferred_base: placement.used_preferred_base,
                entry_point,
            }),
            Err(source) => {
                *memory = before;
                return Err(ModuleMemoryError::MappingFailed {
                    image_index,
                    source,
                });
            }
        }
    }
    Ok(reservations)
}

fn validate_complete_plan(
    images: &[ExplicitModuleImage<'_>],
    plan: &ModuleLayoutPlan,
) -> Result<Vec<ImagePlacement>, ModuleMemoryError> {
    let positions = validate_indexes(images.len(), plan)?;
    let mut placements = Vec::with_capacity(images.len());

    for (image_index, image) in images.iter().enumerate() {
        let position = positions[image_index].expect("the exact-one index check passed");
        let planned_image = &plan.images()[position];
        let placement = match planned_image.outcome {
            ImageLayoutOutcome::Placed(placement) => placement,
            ImageLayoutOutcome::Failed(_) => {
                return Err(ModuleMemoryError::MissingPlacement { image_index });
            }
        };
        if planned_image.descriptor != ModuleImageDescriptor::from_parsed_pe(image.pe) {
            return Err(ModuleMemoryError::DescriptorMismatch { image_index });
        }
        placements.push(placement);
    }

    reject_overlapping_placements(&placements)?;
    Ok(placements)
}

fn validate_indexes(
    module_images: usize,
    plan: &ModuleLayoutPlan,
) -> Result<Vec<Option<usize>>, ModuleMemoryError> {
    if plan.images().len() != module_images {
        return Err(ModuleMemoryError::PlanInputMismatch {
            plan_images: plan.images().len(),
            module_images,
        });
    }

    let mut positions = vec![None; module_images];
    for (plan_position, image) in plan.images().iter().enumerate() {
        if image.index >= module_images || positions[image.index].is_some() {
            return Err(ModuleMemoryError::InvalidPlanIndex {
                plan_position,
                image_index: image.index,
            });
        }
        positions[image.index] = Some(plan_position);
    }
    Ok(positions)
}

fn reject_overlapping_placements(placements: &[ImagePlacement]) -> Result<(), ModuleMemoryError> {
    let mut ordered: Vec<_> = placements.iter().copied().enumerate().collect();
    ordered.sort_by_key(|&(index, placement)| (placement.base, placement.end, index));

    for pair in ordered.windows(2) {
        let (first_index, first) = pair[0];
        let (second_index, second) = pair[1];
        if second.base >= first.end {
            continue;
        }
        return Err(ModuleMemoryError::OverlappingPlacements {
            first_index,
            first: AddressInterval::new(first.base, first.end),
            second_index,
            second: AddressInterval::new(second.base, second.end),
            overlap: AddressInterval::new(first.base.max(second.base), first.end.min(second.end)),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(base: u64, end: u64) -> ImagePlacement {
        ImagePlacement {
            base,
            end,
            used_preferred_base: false,
        }
    }

    #[test]
    fn placement_overlap_check_uses_half_open_intervals() {
        let adjacent = [placement(0x1000, 0x2000), placement(0x2000, 0x3000)];
        assert_eq!(reject_overlapping_placements(&adjacent), Ok(()));

        let overlapping = [placement(0x1000, 0x2001), placement(0x2000, 0x3000)];
        assert_eq!(
            reject_overlapping_placements(&overlapping),
            Err(ModuleMemoryError::OverlappingPlacements {
                first_index: 0,
                first: AddressInterval::new(0x1000, 0x2001),
                second_index: 1,
                second: AddressInterval::new(0x2000, 0x3000),
                overlap: AddressInterval::new(0x2000, 0x2001),
            })
        );
    }

    #[test]
    fn placement_overlap_check_is_independent_of_input_order() {
        let unordered = [placement(0x3000, 0x4000), placement(0x1000, 0x3001)];
        assert_eq!(
            reject_overlapping_placements(&unordered),
            Err(ModuleMemoryError::OverlappingPlacements {
                first_index: 1,
                first: AddressInterval::new(0x1000, 0x3001),
                second_index: 0,
                second: AddressInterval::new(0x3000, 0x4000),
                overlap: AddressInterval::new(0x3000, 0x3001),
            })
        );
    }
}
