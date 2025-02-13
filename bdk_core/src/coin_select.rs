use bitcoin::{Transaction, TxOut};

use crate::{BTreeSet, Vec};

const TXIN_BASE_WEIGHT: u32 = (32 + 4 + 4 + 1) * 4;

#[derive(Debug, Clone)]
pub struct CoinSelector {
    candidates: Vec<WeightedValue>,
    selected: BTreeSet<usize>,
    opts: CoinSelectorOpt,
}

#[derive(Debug, Clone, Copy)]
pub struct WeightedValue {
    pub value: u64,
    pub weight: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct CoinSelectorOpt {
    /// The value we need to select.
    pub target_value: u64,
    /// The feerate we should try and achieve in sats per weight unit.
    pub target_feerate: f32,
    /// The minimum absolute fee.
    pub min_absolute_fee: u64,
    /// The weight of the template transaction including fixed inputs and outputs.
    pub base_weight: u32,
    /// The weight of the drain (change) output.
    pub drain_weight: u32,
    /// The input value of the template transaction.
    pub starting_input_value: u64,
}

impl CoinSelectorOpt {
    pub fn from_weights(base_weight: u32, drain_weight: u32) -> Self {
        Self {
            target_value: 0,
            // by defualt 1 sat per byte (i.e. 4 per wu)
            target_feerate: 4.0,
            min_absolute_fee: 0,
            base_weight,
            drain_weight,
            starting_input_value: 0,
        }
    }

    pub fn fund_outputs(txouts: &[TxOut], drain_weight: u32) -> Self {
        let tx = Transaction {
            input: vec![],
            version: 1,
            lock_time: 0,
            output: txouts.to_vec(),
        };
        Self {
            target_value: txouts.iter().map(|txout| txout.value).sum(),
            ..Self::from_weights(tx.weight() as u32, drain_weight)
        }
    }
}

impl CoinSelector {
    pub fn candidates(&self) -> &[WeightedValue] {
        &self.candidates
    }

    pub fn new(candidates: Vec<WeightedValue>, opts: CoinSelectorOpt) -> Self {
        Self {
            candidates,
            selected: Default::default(),
            opts,
        }
    }

    pub fn select(&mut self, index: usize) {
        assert!(index < self.candidates.len());
        self.selected.insert(index);
    }

    pub fn current_weight(&self) -> u32 {
        self.opts.base_weight
            + self
                .selected()
                .map(|(_, wv)| wv.weight + TXIN_BASE_WEIGHT)
                .sum::<u32>()
    }

    pub fn selected(&self) -> impl Iterator<Item = (usize, WeightedValue)> + '_ {
        self.selected
            .iter()
            .map(|index| (*index, self.candidates.get(*index).unwrap().clone()))
    }

    pub fn unselected(&self) -> Vec<usize> {
        let all_indexes = (0..self.candidates.len()).collect::<BTreeSet<_>>();
        all_indexes.difference(&self.selected).cloned().collect()
    }

    pub fn all_selected(&self) -> bool {
        self.selected.len() == self.candidates.len()
    }

    pub fn select_until_finished(&mut self) -> Option<Selection> {
        let mut selection = None;

        for next_unselected in self.unselected() {
            selection = self.finish();

            if selection.is_some() {
                break;
            }
            self.select(next_unselected)
        }

        selection
    }

    pub fn current_value(&self) -> u64 {
        self.opts.starting_input_value + self.selected().map(|(_, wv)| wv.value).sum::<u64>()
    }

    pub fn finish(&self) -> Option<Selection> {
        let base_weight = self.current_weight();

        if self.current_value() < self.opts.target_value {
            return None;
        }

        let inputs_minus_outputs = self.current_value() - self.opts.target_value;

        // check fee rate satisfied
        let feerate_without_change = inputs_minus_outputs as f32 / base_weight as f32;

        // we simply don't have enough fee to acheieve the feerate
        if feerate_without_change < self.opts.target_feerate {
            return None;
        }

        if inputs_minus_outputs < self.opts.min_absolute_fee {
            return None;
        }

        let weight_with_change = base_weight + self.opts.drain_weight;
        let target_fee_with_change = ((self.opts.target_feerate * weight_with_change as f32).ceil()
            as u64)
            .max(self.opts.min_absolute_fee);
        let target_fee_without_change = ((self.opts.target_feerate * base_weight as f32).ceil()
            as u64)
            .max(self.opts.min_absolute_fee);

        let (excess, use_change) = match inputs_minus_outputs.checked_sub(target_fee_with_change) {
            Some(excess) => (excess, true),
            None => {
                let implied_output_value = self.current_value() - target_fee_without_change;
                match implied_output_value.checked_sub(self.opts.target_value) {
                    Some(excess) => (excess, false),
                    None => return None,
                }
            }
        };

        let (total_weight, fee) = if use_change {
            (weight_with_change, target_fee_with_change)
        } else {
            (base_weight, target_fee_without_change)
        };

        Some(Selection {
            selected: self.selected.clone(),
            excess,
            use_change,
            total_weight,
            fee,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Selection {
    pub selected: BTreeSet<usize>,
    pub excess: u64,
    pub fee: u64,
    pub use_change: bool,
    pub total_weight: u32,
}

impl Selection {
    pub fn apply_selection<'a, T>(
        &'a self,
        candidates: &'a [T],
    ) -> impl Iterator<Item = &'a T> + 'a {
        self.selected.iter().map(|i| &candidates[*i])
    }
}
