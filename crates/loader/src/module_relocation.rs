// SPDX-License-Identifier: GPL-3.0-or-later

//! Transactional batch relocation for explicit PE32+ module images.
//!
//! This narrow seam connects the logical reservations returned by
//! [`map_module_batch`] to the existing single-image [`apply_relocations`]
//! primitive. It checks that the reservation metadata belongs to the exact
//! ordered input batch before touching memory, applies relocations for every
//! image in input order at its reservation base and returns the per-image
//! reports beside the reservation records they were applied against.
//!
//! A complete reservation batch has exactly one record for every input image
//! index, with no missing, duplicate or extra indexes, and every record must
//! describe a nonzero half-open `[base, end)` span. These checks are defensive
//! even though [`map_module_batch`] currently produces exactly that shape: the
//! memory boundary must not depend on a caller composing the wrong metadata
//! for a batch. Nothing else is inferred from the spans. This layer does not
//! check base alignment, agreement with the image extent or overlap between
//! reservations; it only uses each base as the `loaded_base` argument of
//! [`apply_relocations`], whose own validation decides whether the relocation
//! directory and targets are admissible.
//!
//! Rollback is deliberately exact and deliberately expensive, matching the
//! batch mapping layer. For a nonempty batch, after the whole metadata
//! validation passes, the batch clones the entire [`GuestMemory`] once. If
//! applying relocations fails for any image, that clone is assigned back,
//! restoring the prior address space and its metadata rather than merely
//! reverting the fixups of earlier images. Time cost scales with the batch
//! length times the committed guest bytes, while peak host memory includes
//! the live address space and the batch snapshot. This is not the final
//! memory-ownership or efficient reservation design.
//!
//! Images loaded at their preferred base have a zero relocation delta, and
//! images without a relocation directory apply no fixups; both outcomes are
//! successful reports through the existing relocation semantics, not errors.
//! This layer performs no import or IAT binding, API-set resolution, service
//! registration, exception or TLS initialization, `ImageBase` field patching,
//! unloading, reference counting or guest execution.

use std::fmt;

use crate::memory::GuestMemory;
use crate::module_memory::{ExplicitModuleImage, ModuleReservation};
use crate::reloc::{RelocationError, RelocationReport, apply_relocations};

/// The relocation outcome for one image: the reservation the fixups were
/// applied against beside the primitive's own report. The report is honest
/// about a zero delta or a missing relocation directory: both produce no
/// applied fixups without failing the batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleRelocationOutcome {
    pub reservation: ModuleReservation,
    pub report: RelocationReport,
}

/// Errors that prevent a module batch from being relocated as one
/// transaction. Every image-specific failure carries the input `image_index`.
#[derive(Debug, PartialEq, Eq)]
pub enum ModuleRelocationError {
    /// The reservation slice does not have one record for each input image.
    /// Because the exact-one rule is checked before memory mutation, too few
    /// and too many records are both batch-shape errors rather than partial
    /// relocations.
    ReservationInputMismatch {
        reservations: usize,
        module_images: usize,
    },
    /// A reservation record index is outside the input batch or repeats an
    /// earlier record. A repeat necessarily leaves another input index
    /// missing.
    InvalidReservationIndex {
        reservation_position: usize,
        image_index: usize,
    },
    /// The record for an input image is not a nonzero half-open span:
    /// `end` is not greater than `base`.
    InvalidReservationSpan {
        image_index: usize,
        base: u64,
        end: u64,
    },
    /// The image's own relocation application failed. The batch restored the
    /// exact prior [`GuestMemory`] snapshot before returning this error, so
    /// fixups already applied to earlier images are reverted as well.
    RelocationFailed {
        image_index: usize,
        source: RelocationError,
    },
}

impl fmt::Display for ModuleRelocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReservationInputMismatch {
                reservations,
                module_images,
            } => write!(
                f,
                "{reservations} relocation reservations for {module_images} module images"
            ),
            Self::InvalidReservationIndex {
                reservation_position,
                image_index,
            } => write!(
                f,
                "reservation record {reservation_position} has missing, repeated or out-of-batch image index {image_index}"
            ),
            Self::InvalidReservationSpan {
                image_index,
                base,
                end,
            } => write!(
                f,
                "reservation for image {image_index} is not a nonzero half-open span: [{base:#x}, {end:#x})"
            ),
            Self::RelocationFailed {
                image_index,
                source,
            } => write!(f, "image {image_index} failed relocation: {source}"),
        }
    }
}

impl std::error::Error for ModuleRelocationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RelocationFailed { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Apply base relocations to a complete explicit module batch as one
/// transaction.
///
/// Validation of the complete reservation metadata finishes before the batch
/// snapshot is made and before any fixup can mutate memory. Images are then
/// relocated in input order through [`apply_relocations`] at their reservation
/// bases. A failure at any image assigns the pre-batch snapshot back to
/// `memory`, so the rebased bytes of successful earlier images do not remain.
///
/// On success the outcomes are returned in input order, each pairing the
/// reservation with its relocation report. The batch memory state is the
/// pre-batch state plus the applied fixups; the images stay mapped exactly as
/// they were when this batch started.
pub fn relocate_module_batch(
    images: &[ExplicitModuleImage<'_>],
    reservations: &[ModuleReservation],
    memory: &mut GuestMemory,
) -> Result<Vec<ModuleRelocationOutcome>, ModuleRelocationError> {
    let ordered = validate_reservations(images.len(), reservations)?;
    if images.is_empty() {
        return Ok(Vec::new());
    }

    let before = memory.clone();
    let mut relocated = Vec::with_capacity(images.len());
    for (image_index, image) in images.iter().enumerate() {
        let reservation = ordered[image_index];
        match apply_relocations(image.pe, image.bytes, reservation.base, memory) {
            Ok(report) => relocated.push(ModuleRelocationOutcome {
                reservation,
                report,
            }),
            Err(source) => {
                *memory = before;
                return Err(ModuleRelocationError::RelocationFailed {
                    image_index,
                    source,
                });
            }
        }
    }
    Ok(relocated)
}

/// Check the exact-one index rule before any span is inspected, then return
/// the reservations reordered into input order. The caller's slice order does
/// not matter because records are matched by their `image_index` field.
fn validate_reservations(
    module_images: usize,
    reservations: &[ModuleReservation],
) -> Result<Vec<ModuleReservation>, ModuleRelocationError> {
    if reservations.len() != module_images {
        return Err(ModuleRelocationError::ReservationInputMismatch {
            reservations: reservations.len(),
            module_images,
        });
    }

    let mut slots = vec![None; module_images];
    for (reservation_position, reservation) in reservations.iter().enumerate() {
        if reservation.image_index >= module_images || slots[reservation.image_index].is_some() {
            return Err(ModuleRelocationError::InvalidReservationIndex {
                reservation_position,
                image_index: reservation.image_index,
            });
        }
        slots[reservation.image_index] = Some(*reservation);
    }

    let mut ordered = Vec::with_capacity(module_images);
    for (image_index, slot) in slots.into_iter().enumerate() {
        let reservation = slot.expect("the exact-one index check passed");
        if reservation.end <= reservation.base {
            return Err(ModuleRelocationError::InvalidReservationSpan {
                image_index,
                base: reservation.base,
                end: reservation.end,
            });
        }
        ordered.push(reservation);
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reservation(image_index: usize, base: u64, end: u64) -> ModuleReservation {
        ModuleReservation {
            image_index,
            base,
            end,
            used_preferred_base: true,
            entry_point: None,
        }
    }

    #[test]
    fn validation_accepts_reservations_in_any_slice_order() {
        let ordered = validate_reservations(
            2,
            &[
                reservation(1, 0x2000, 0x3000),
                reservation(0, 0x1000, 0x2000),
            ],
        )
        .expect("one valid record per image");
        assert_eq!(ordered[0].image_index, 0);
        assert_eq!(ordered[1].image_index, 1);
    }

    #[test]
    fn validation_rejects_repeated_and_out_of_batch_indexes() {
        let repeated = validate_reservations(
            2,
            &[
                reservation(0, 0x1000, 0x2000),
                reservation(0, 0x2000, 0x3000),
            ],
        );
        assert_eq!(
            repeated,
            Err(ModuleRelocationError::InvalidReservationIndex {
                reservation_position: 1,
                image_index: 0,
            })
        );

        let out_of_batch = validate_reservations(1, &[reservation(1, 0x1000, 0x2000)]);
        assert_eq!(
            out_of_batch,
            Err(ModuleRelocationError::InvalidReservationIndex {
                reservation_position: 0,
                image_index: 1,
            })
        );
    }

    #[test]
    fn validation_rejects_empty_and_reversed_spans_in_input_order() {
        for (base, end) in [(0x1000, 0x1000), (0x2000, 0x1000)] {
            let result = validate_reservations(
                2,
                &[reservation(1, 0x4000, 0x5000), reservation(0, base, end)],
            );
            assert_eq!(
                result,
                Err(ModuleRelocationError::InvalidReservationSpan {
                    image_index: 0,
                    base,
                    end,
                })
            );
        }
    }
}
