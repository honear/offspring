//! Shared `IEnumExplorerCommand` cursor used by every flyout we
//! expose (the top-level Offspring root and the nested Tools
//! sub-flyout). Walks a precomputed `Vec<IExplorerCommand>` once,
//! handing entries back in chunks the way Explorer's enumeration
//! protocol expects.
//!
//! Lives in its own module purely so both `RootCommand` and
//! `ToolsCommand` can construct one without duplicating the macro-
//! generated `IEnumExplorerCommand_Impl` boilerplate. There's no
//! per-flyout state worth specialising — it's just a Vec + cursor.

use std::cell::Cell;

use windows::core::*;
use windows::Win32::Foundation::{S_FALSE, S_OK};
use windows::Win32::UI::Shell::{IEnumExplorerCommand, IEnumExplorerCommand_Impl, IExplorerCommand};

#[implement(IEnumExplorerCommand)]
pub struct SubEnum {
    pub items: Vec<IExplorerCommand>,
    pub cursor: Cell<usize>,
}

impl SubEnum {
    /// Build an `IEnumExplorerCommand` interface object that walks
    /// `items` in order. Always starts the cursor at zero — Explorer
    /// can `Reset()` it later if needed.
    pub fn into_iface(items: Vec<IExplorerCommand>) -> IEnumExplorerCommand {
        SubEnum {
            items,
            cursor: Cell::new(0),
        }
        .into()
    }
}

impl IEnumExplorerCommand_Impl for SubEnum_Impl {
    fn Next(
        &self,
        celt: u32,
        rgelt: *mut Option<IExplorerCommand>,
        pceltfetched: *mut u32,
    ) -> HRESULT {
        let start = self.cursor.get();
        let end = (start + celt as usize).min(self.items.len());
        let count = end - start;
        unsafe {
            for i in 0..count {
                *rgelt.add(i) = Some(self.items[start + i].clone());
            }
            if !pceltfetched.is_null() {
                *pceltfetched = count as u32;
            }
        }
        self.cursor.set(end);
        if count == celt as usize {
            S_OK
        } else {
            S_FALSE
        }
    }

    fn Skip(&self, celt: u32) -> Result<()> {
        let next = self.cursor.get().saturating_add(celt as usize);
        if next > self.items.len() {
            self.cursor.set(self.items.len());
            Err(S_FALSE.into())
        } else {
            self.cursor.set(next);
            Ok(())
        }
    }

    fn Reset(&self) -> Result<()> {
        self.cursor.set(0);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumExplorerCommand> {
        let clone = SubEnum {
            items: self.items.clone(),
            cursor: Cell::new(self.cursor.get()),
        };
        Ok(clone.into())
    }
}
