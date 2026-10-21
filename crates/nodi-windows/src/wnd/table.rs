//! Real Rich Edit tables (not ASCII art): insert, add/delete rows and
//! columns.
//!
//! In Rich Edit's text a table row is `U+FFF9 \r` cells… `U+FFFB \r`, each cell
//! ending with `U+0007`. Row boundaries are found from those marks (nesting
//! aware); row/cell parameters go through `EM_GETTABLEPARMS`,
//! `EM_INSERTTABLE` and `EM_SETTABLEPARMS` (Windows 8+).

use std::mem::size_of;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW;

use crate::wnd::editor;
use crate::wnd::format::{full_width_twips, rgb};

const WM_USER: u32 = 0x0400;
const EM_INSERTTABLE: u32 = WM_USER + 232;
const EM_GETTABLEPARMS: u32 = WM_USER + 265;
const EM_SETTABLEPARMS: u32 = WM_USER + 307;
const MAX_CELLS: usize = 63;

const ROW_START: u16 = 0xFFF9;
const ROW_END: u16 = 0xFFFB;
const CELL_END: u16 = 0x0007;

/// `TABLEROWPARMS` (richedit.h, `pack(4)`): 28 bytes.
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
struct RowParms {
    cb_row: u8,
    cb_cell: u8,
    c_cell: u8,
    c_row: u8,
    dx_cell_margin: i32,
    dx_indent: i32,
    dy_height: i32,
    /// nAlignment:3, fRTL:1, fKeep:1, fKeepFollow:1, fWrap:1, fIdentCells:1
    flags: u32,
    cp_start_row: i32,
    b_table_level: u8,
    i_cell: u8,
}

/// `TABLECELLPARMS` (richedit.h): 40 bytes.
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
struct CellParms {
    dx_width: i32,
    /// nVertAlign:2, fMergeTop:1, fMergePrev:1, fVertical:1, fMergeStart:1, fMergeCont:1
    flags: u16,
    w_shading: u16,
    dx_brdr_left: i16,
    dy_brdr_top: i16,
    dx_brdr_right: i16,
    dy_brdr_bottom: i16,
    cr_brdr_left: u32,
    cr_brdr_top: u32,
    cr_brdr_right: u32,
    cr_brdr_bottom: u32,
    cr_back_pat: u32,
    cr_fore_pat: u32,
}

const _: () = assert!(size_of::<RowParms>() == 28);
const _: () = assert!(size_of::<CellParms>() == 40);

fn row_parms(cells: usize, rows: usize, cp_start_row: i32) -> RowParms {
    RowParms {
        cb_row: size_of::<RowParms>() as u8,
        cb_cell: size_of::<CellParms>() as u8,
        c_cell: cells as u8,
        c_row: rows as u8,
        dx_cell_margin: 108,
        cp_start_row,
        ..RowParms::default()
    }
}

fn plain_cell(width: i32) -> CellParms {
    let border = rgb(160, 160, 160);
    CellParms {
        dx_width: width,
        dx_brdr_left: 10,
        dy_brdr_top: 10,
        dx_brdr_right: 10,
        dy_brdr_bottom: 10,
        cr_brdr_left: border,
        cr_brdr_top: border,
        cr_brdr_right: border,
        cr_brdr_bottom: border,
        cr_back_pat: rgb(255, 255, 255),
        ..CellParms::default()
    }
}

/// Insert > Table: `rows`×`cols` empty cells spanning the editor width.
pub fn insert(editor: HWND, rows: usize, cols: usize) -> Result<(), String> {
    let rows = rows.clamp(1, 255);
    let cols = cols.clamp(1, MAX_CELLS);
    let at = crate::wnd::format::prepare_block(editor);
    let width = full_width_twips(editor) / cols as i32;
    let cells = vec![plain_cell(width.max(400)); cols];
    let row = row_parms(cols, rows, -1);
    // SAFETY: `row` and `cells` (one entry per cell) outlive the call.
    let result = unsafe {
        SendMessageW(
            editor,
            EM_INSERTTABLE,
            &row as *const RowParms as usize,
            cells.as_ptr() as isize,
        )
    };
    hresult(result, "insert the table")?;
    // Despite the documentation the caret ends up after the table; put it in
    // the first cell (row start U+FFF9 + its paragraph mark).
    editor::set_selection(editor, at + 2, at + 2);
    Ok(())
}

fn hresult(value: isize, what: &str) -> Result<(), String> {
    if value == 0 {
        Ok(())
    } else {
        Err(format!(
            "Rich Edit could not {what} (HRESULT 0x{:08X}).",
            value as u32
        ))
    }
}

/// One table row in document coordinates: `start` is its U+FFF9, `end` its
/// U+FFFB (the row occupies `start..end + 2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Row {
    start: i32,
    end: i32,
    depth: usize,
}

/// Every row in the document, outer rows before the rows nested inside them.
fn rows(units: &[u16]) -> Vec<Row> {
    let mut open: Vec<i32> = Vec::new();
    let mut found = Vec::new();
    for (index, unit) in units.iter().enumerate() {
        match *unit {
            ROW_START => open.push(index as i32),
            ROW_END => {
                if let Some(start) = open.pop() {
                    found.push(Row {
                        start,
                        end: index as i32,
                        depth: open.len(),
                    });
                }
            }
            _ => {}
        }
    }
    found.sort_by_key(|row| (row.start, row.depth));
    found
}

/// Innermost row that contains `pos`.
fn row_at(rows: &[Row], pos: i32) -> Option<Row> {
    rows.iter()
        .filter(|row| row.start <= pos && pos <= row.end)
        .max_by_key(|row| row.depth)
        .copied()
}

/// Zero-based cell index of `pos` inside `row` (ignores nested rows).
fn cell_index(units: &[u16], row: Row, pos: i32) -> usize {
    let mut depth = 0usize;
    let mut cell = 0usize;
    for index in (row.start + 1)..pos.min(row.end) {
        match units[index as usize] {
            ROW_START => depth += 1,
            ROW_END => depth = depth.saturating_sub(1),
            CELL_END if depth == 0 => cell += 1,
            _ => {}
        }
    }
    cell
}

/// First row of the table that `row` belongs to (same depth, contiguous).
fn table_start(rows: &[Row], row: Row) -> Row {
    let mut first = row;
    while let Some(previous) = rows
        .iter()
        .find(|candidate| candidate.depth == first.depth && candidate.end + 2 == first.start)
    {
        first = *previous;
    }
    first
}

fn document(editor: HWND) -> Vec<u16> {
    editor::text_units(editor, 0, editor::text_length(editor))
}

fn get_params(editor: HWND, row: Row) -> Option<(RowParms, Vec<CellParms>)> {
    let mut params = row_parms(MAX_CELLS, 1, row.start);
    let mut cells = vec![CellParms::default(); MAX_CELLS];
    // SAFETY: room for MAX_CELLS cells; both buffers outlive the call.
    let result = unsafe {
        SendMessageW(
            editor,
            EM_GETTABLEPARMS,
            &mut params as *mut RowParms as usize,
            cells.as_mut_ptr() as isize,
        )
    };
    if result != 0 {
        return None;
    }
    let count = usize::from(params.c_cell).min(MAX_CELLS);
    cells.truncate(count);
    Some((params, cells))
}

/// Context needed by every row/column operation at the caret.
struct Here {
    row: Row,
    cell: usize,
    params: RowParms,
    cells: Vec<CellParms>,
    all_rows: Vec<Row>,
}

fn here(editor: HWND) -> Result<Here, String> {
    let units = document(editor);
    let all_rows = rows(&units);
    let (caret, _) = editor::selection(editor);
    let row = row_at(&all_rows, caret).ok_or("The caret is not inside a table.")?;
    let cell = cell_index(&units, row, caret);
    let (params, cells) =
        get_params(editor, row).ok_or("Rich Edit did not report the table layout.")?;
    Ok(Here {
        row,
        cell,
        params,
        cells,
        all_rows,
    })
}

pub fn insert_row(editor: HWND, below: bool) -> Result<(), String> {
    let here = here(editor)?;
    let at = if below {
        here.row.end + 2
    } else {
        here.row.start
    };
    let mut params = here.params;
    params.c_row = 1;
    params.cp_start_row = at;
    params.cb_row = size_of::<RowParms>() as u8;
    params.cb_cell = size_of::<CellParms>() as u8;
    // SAFETY: `params` and the cell array outlive the call.
    let result = unsafe {
        SendMessageW(
            editor,
            EM_INSERTTABLE,
            &params as *const RowParms as usize,
            here.cells.as_ptr() as isize,
        )
    };
    hresult(result, "insert the row")?;
    editor::set_selection(editor, at + 2, at + 2);
    Ok(())
}

pub fn delete_row(editor: HWND) -> Result<(), String> {
    let here = here(editor)?;
    editor::set_selection(editor, here.row.start, here.row.end + 2);
    clear_selection(editor);
    Ok(())
}

/// Deletes the selection like the Del key. `EM_REPLACESEL("")` on a middle
/// table row leaves an empty paragraph that splits the table in two;
/// `WM_CLEAR` removes the row cleanly.
fn clear_selection(editor: HWND) {
    const WM_CLEAR: u32 = 0x0303;
    // SAFETY: plain message to our Rich Edit.
    unsafe {
        SendMessageW(editor, WM_CLEAR, 0, 0);
    }
}

/// Adds a column right (or left) of the caret's column in every row.
pub fn insert_column(editor: HWND, right: bool) -> Result<(), String> {
    let here = here(editor)?;
    if here.cells.len() >= MAX_CELLS {
        return Err("A table row can have at most 63 cells.".to_owned());
    }
    let index = if right { here.cell + 1 } else { here.cell };
    let source = here.cells[here.cell.min(here.cells.len() - 1)];
    let mut cells = here.cells.clone();
    cells.insert(index.min(cells.len()), source);
    set_cells(editor, &here, cells, index)
}

pub fn delete_column(editor: HWND) -> Result<(), String> {
    let here = here(editor)?;
    if here.cells.len() <= 1 {
        let first = table_start(&here.all_rows, here.row);
        let mut last = here.row;
        while let Some(next) = here
            .all_rows
            .iter()
            .find(|row| row.depth == last.depth && row.start == last.end + 2)
        {
            last = *next;
        }
        editor::set_selection(editor, first.start, last.end + 2);
        clear_selection(editor);
        return Ok(());
    }
    let mut cells = here.cells.clone();
    cells.remove(here.cell.min(cells.len() - 1));
    set_cells(editor, &here, cells, here.cell)
}

fn set_cells(editor: HWND, here: &Here, cells: Vec<CellParms>, index: usize) -> Result<(), String> {
    let first = table_start(&here.all_rows, here.row);
    let mut params = here.params;
    params.cb_row = size_of::<RowParms>() as u8;
    params.cb_cell = size_of::<CellParms>() as u8;
    params.c_cell = cells.len() as u8;
    params.c_row = 0xFF; // -1: every row to the end of the table
    params.i_cell = index as u8;
    params.cp_start_row = first.start;
    // SAFETY: `params` and `cells` outlive the call.
    let result = unsafe {
        SendMessageW(
            editor,
            EM_SETTABLEPARMS,
            &params as *const RowParms as usize,
            cells.as_ptr() as isize,
        )
    };
    hresult(result, "change the columns")
}

/// True when the caret is inside any table row.
pub fn caret_in_table(editor: HWND) -> bool {
    editor::in_table(editor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spec: &str) -> Vec<u16> {
        // R = row start, E = row end, C = cell end
        spec.chars()
            .map(|ch| match ch {
                'R' => ROW_START,
                'E' => ROW_END,
                'C' => CELL_END,
                '|' => 0x000D,
                other => other as u16,
            })
            .collect()
    }

    #[test]
    fn finds_rows_and_cells() {
        let units = text("x|R|aCbC|E|R|cCdC|E|y");
        let all = rows(&units);
        assert_eq!(all.len(), 2);
        let second = row_at(&all, 12).unwrap();
        assert_eq!(second.start, all[1].start);
        assert_eq!(cell_index(&units, second, second.start + 2), 0);
        let d = units.iter().position(|unit| *unit == 'd' as u16).unwrap() as i32;
        assert_eq!(cell_index(&units, second, d), 1);
        assert_eq!(table_start(&all, second), all[0]);
    }

    #[test]
    fn nested_row_is_innermost() {
        let units = text("R|aR|bC|ECE|");
        let all = rows(&units);
        assert_eq!(all.len(), 2);
        let b = units.iter().position(|unit| *unit == 'b' as u16).unwrap() as i32;
        assert_eq!(row_at(&all, b).unwrap().depth, 1);
    }
}
