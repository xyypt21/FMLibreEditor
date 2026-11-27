//! Reads the transfers and loans the game has agreed for a later day: the offers in state 19 of the transfer manager's
//! list (see `arranged.rs` and docs/recon/fm27-transfers.md). Only reads memory.

use fmlibre_mem::is_user_pointer;
use serde::Serialize;

use crate::date::FmDate;
use crate::players::{PlayerReader, PlayerSummary};
use crate::session::Session;
use crate::transfer::MoveError;

/// An offer's state once it is agreed and waiting for the day of the move.
const AGREED: u8 = 19;

/// An offer's kind for a loan (see [`crate::profile::TransfersLayout::offer_kind`]).
const LOAN: u8 = 1;

/// A transfer or loan the game has agreed for a later day.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgreedMove {
    /// The player, read from the object the offer names.
    pub player: PlayerSummary,
    pub uid: u32,
    pub name: String,
    /// The team he joins, and the team that employs him now (0 when he has none).
    pub buyer_team: u64,
    pub seller_team: u64,
    /// The day the game makes the move.
    pub move_day: Option<FmDate>,
    /// Whether the move is a loan.
    pub loan: bool,
}

impl Session {
    /// The transfers and loans the game has agreed and will make on their day (offers in state 19). Only reads memory;
    /// a build whose layout does not place the transfers has none.
    pub fn agreed_moves(&self) -> Result<Vec<AgreedMove>, MoveError> {
        let Some(t) = self.layout.transfers.as_ref() else {
            return Ok(Vec::new());
        };
        // A build whose profile does not place the manager has no agreed moves to read; that is not an error, and the
        // editor does not look for the manager elsewhere.
        if self.profile.as_ref().and_then(|profile| profile.transfer_manager).is_none() {
            return Ok(Vec::new());
        }
        let manager = self.transfer_manager(t)?;
        let offers = self.manager_list(manager + t.offers)?;
        // The offers are the manager's own list, nearly all of them not agreed yet: check an offer's class only for the
        // few that are, as finding the classes by name over the whole plugin takes many seconds.
        let transfer_class = crate::rtti::mangled(&t.offer_class);
        let loan_class = t.loan.as_ref().map(|loan| crate::rtti::mangled(&loan.offer_class));
        let identity = self.layout.player.identity();
        let today = self.game_date();
        let mut reader = PlayerReader::new(&self.process, &self.layout, today);
        let mut moves: Vec<AgreedMove> = Vec::new();
        for offer in offers.elements(&self.process)? {
            if !is_user_pointer(offer) {
                continue;
            }
            let Ok(state) = self.process.read_bytes(offer + t.offer_state, 1) else {
                continue;
            };
            if state[0] != AGREED {
                continue;
            }
            // Where the platform's class names can be read, the offer must be one of a transfer or a loan; where they
            // cannot (no Mac save has been read for it), the state and the fields tell.
            if let Some(name) = crate::rtti::class_of(&self.process, &self.plugin, offer)
                && name != transfer_class
                && loan_class.as_ref() != Some(&name)
            {
                continue;
            }
            let Ok(person) = self.process.read_u64(offer + t.offer_person) else {
                continue;
            };
            let Ok(buyer) = self.process.read_u64(offer + t.offer_buyer) else {
                continue;
            };
            // The offer names the player's identity block, which sits `identity` bytes into his object.
            let Some(object) = person.checked_sub(identity).filter(|_| is_user_pointer(person)) else {
                continue;
            };
            if !is_user_pointer(buyer) {
                continue;
            }
            let seller = self.process.read_u64(offer + t.offer_seller).unwrap_or(0);
            let word = self.process.read_u32(offer + t.offer_move_day).unwrap_or(0);
            let move_day = FmDate::from_raw((word & 0xFFFF) as u16, (word >> 16) as u16);
            // An offer is waiting only while its day is still ahead: the game makes it on the day (state 20), and an
            // old entry left agreed is not a move to come.
            if let Some(today) = today
                && move_day.is_none_or(|day| day <= today)
            {
                continue;
            }
            let kind = self.process.read_bytes(offer + t.offer_kind, 1).map(|byte| byte[0]).unwrap_or(0);
            let Some(player) = reader.read(object).filter(|player| player.uid != 0) else {
                continue;
            };
            let (uid, name) = (player.uid, player.name.clone());
            moves.push(AgreedMove {
                player,
                uid,
                name,
                buyer_team: buyer,
                seller_team: seller,
                move_day,
                loan: kind == LOAN,
            });
        }
        // A player can have more than one agreed offer listed; keep the move on the latest day.
        moves.sort_by_key(|moved| (moved.uid, std::cmp::Reverse(moved.move_day)));
        moves.dedup_by_key(|moved| moved.uid);
        Ok(moves)
    }
}
