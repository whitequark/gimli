use common::{
    DebugAddrBase, DebugAddrIndex, DebugLocListsBase, DebugLocListsIndex, DebugRngListsBase,
    DebugRngListsIndex, DebugStrOffsetsBase, Encoding, LocationListsOffset, RangeListsOffset,
};
use constants;
use read::{
    Abbreviations, AttributeValue, CompilationUnitHeader, CompilationUnitHeadersIter, DebugAbbrev,
    DebugAddr, DebugInfo, DebugLine, DebugLineStr, DebugStr, DebugStrOffsets, DebugTypes,
    EntriesCursor, Error, IncompleteLineProgram, LocListIter, LocationLists, RangeLists, Reader,
    ReaderOffset, Result, RngListIter, TypeUnitHeader, TypeUnitHeadersIter, UnitHeader,
};

/// All of the commonly used DWARF sections, and other common information.
#[derive(Debug, Default)]
pub struct Dwarf<R: Reader> {
    /// The `.debug_abbrev` section.
    pub debug_abbrev: DebugAbbrev<R>,

    /// The `.debug_addr` section.
    pub debug_addr: DebugAddr<R>,

    /// The `.debug_info` section.
    pub debug_info: DebugInfo<R>,

    /// The `.debug_line` section.
    pub debug_line: DebugLine<R>,

    /// The `.debug_line_str` section.
    pub debug_line_str: DebugLineStr<R>,

    /// The `.debug_str` section.
    pub debug_str: DebugStr<R>,

    /// The `.debug_str_offsets` section.
    pub debug_str_offsets: DebugStrOffsets<R>,

    /// The `.debug_str` section for a supplementary object file.
    pub debug_str_sup: DebugStr<R>,

    /// The `.debug_types` section.
    pub debug_types: DebugTypes<R>,

    /// The location lists in the `.debug_loc` and `.debug_loclists` sections.
    pub locations: LocationLists<R>,

    /// The range lists in the `.debug_ranges` and `.debug_rnglists` sections.
    pub ranges: RangeLists<R>,
}

impl<R: Reader> Dwarf<R> {
    /// Iterate the compilation- and partial-units in this
    /// `.debug_info` section.
    ///
    /// Can be [used with
    /// `FallibleIterator`](./index.html#using-with-fallibleiterator).
    #[inline]
    pub fn units(&self) -> CompilationUnitHeadersIter<R> {
        self.debug_info.units()
    }

    /// Iterate the type-units in this `.debug_types` section.
    ///
    /// Can be [used with
    /// `FallibleIterator`](./index.html#using-with-fallibleiterator).
    #[inline]
    pub fn type_units(&self) -> TypeUnitHeadersIter<R> {
        self.debug_types.units()
    }

    /// Parse the abbreviations for a compilation unit.
    // TODO: provide caching of abbreviations
    #[inline]
    pub fn abbreviations(
        &self,
        unit: &CompilationUnitHeader<R, R::Offset>,
    ) -> Result<Abbreviations> {
        unit.abbreviations(&self.debug_abbrev)
    }

    /// Parse the abbreviations for a type unit.
    // TODO: provide caching of abbreviations
    #[inline]
    pub fn type_abbreviations(&self, unit: &TypeUnitHeader<R, R::Offset>) -> Result<Abbreviations> {
        unit.abbreviations(&self.debug_abbrev)
    }

    /// Return an attribute value as a string slice.
    ///
    /// If the attribute value is one of:
    ///
    /// - an inline `DW_FORM_string` string
    /// - a `DW_FORM_strp` reference to an offset into the `.debug_str` section
    /// - a `DW_FORM_strp_sup` reference to an offset into a supplementary
    /// object file
    /// - a `DW_FORM_line_strp` reference to an offset into the `.debug_line_str`
    /// section
    /// - a `DW_FORM_strx` index into the `.debug_str_offsets` entries for the unit
    ///
    /// then return the attribute's string value. Returns an error if the attribute
    /// value does not have a string form, or if a string form has an invalid value.
    pub fn attr_string(
        &self,
        unit: &DwarfUnit<R>,
        attr: AttributeValue<R, R::Offset>,
    ) -> Result<R> {
        match attr {
            AttributeValue::String(string) => Ok(string),
            AttributeValue::DebugStrRef(offset) => self.debug_str.get_str(offset),
            AttributeValue::DebugStrRefSup(offset) => self.debug_str_sup.get_str(offset),
            AttributeValue::DebugLineStrRef(offset) => self.debug_line_str.get_str(offset),
            AttributeValue::DebugStrOffsetsIndex(index) => {
                let offset = self.debug_str_offsets.get_str_offset(
                    unit.header.format(),
                    unit.str_offsets_base,
                    index,
                )?;
                self.debug_str.get_str(offset)
            }
            _ => Err(Error::ExpectedStringAttributeValue),
        }
    }

    /// Return the address at the given index.
    pub fn address(&self, unit: &DwarfUnit<R>, index: DebugAddrIndex<R::Offset>) -> Result<u64> {
        self.debug_addr
            .get_address(unit.encoding().address_size, unit.addr_base, index)
    }

    /// Return the range list offset at the given index.
    pub fn ranges_offset(
        &self,
        unit: &DwarfUnit<R>,
        index: DebugRngListsIndex<R::Offset>,
    ) -> Result<RangeListsOffset<R::Offset>> {
        self.ranges
            .get_offset(unit.encoding(), unit.rnglists_base, index)
    }

    /// Iterate over the `RangeListEntry`s starting at the given offset.
    pub fn ranges(
        &self,
        unit: &DwarfUnit<R>,
        offset: RangeListsOffset<R::Offset>,
    ) -> Result<RngListIter<R>> {
        self.ranges.ranges(
            offset,
            unit.encoding(),
            unit.low_pc,
            &self.debug_addr,
            unit.addr_base,
        )
    }

    /// Try to return an attribute value as a range list offset.
    ///
    /// If the attribute value is one of:
    ///
    /// - a `DW_FORM_sec_offset` reference to the `.debug_ranges` or `.debug_rnglists` sections
    /// - a `DW_FORM_rnglistx` index into the `.debug_rnglists` entries for the unit
    ///
    /// then return the range list offset of the range list.
    /// Returns `None` for other forms.
    pub fn attr_ranges_offset(
        &self,
        unit: &DwarfUnit<R>,
        attr: AttributeValue<R, R::Offset>,
    ) -> Result<Option<RangeListsOffset<R::Offset>>> {
        match attr {
            AttributeValue::RangeListsRef(offset) => Ok(Some(offset)),
            AttributeValue::DebugRngListsIndex(index) => self.ranges_offset(unit, index).map(Some),
            _ => Ok(None),
        }
    }

    /// Try to return an attribute value as a range list entry iterator.
    ///
    /// If the attribute value is one of:
    ///
    /// - a `DW_FORM_sec_offset` reference to the `.debug_ranges` or `.debug_rnglists` sections
    /// - a `DW_FORM_rnglistx` index into the `.debug_rnglists` entries for the unit
    ///
    /// then return an iterator over the entries in the range list.
    /// Returns `None` for other forms.
    pub fn attr_ranges(
        &self,
        unit: &DwarfUnit<R>,
        attr: AttributeValue<R, R::Offset>,
    ) -> Result<Option<RngListIter<R>>> {
        match self.attr_ranges_offset(unit, attr)? {
            Some(offset) => Ok(Some(self.ranges(unit, offset)?)),
            None => Ok(None),
        }
    }

    /// Return the location list offset at the given index.
    pub fn locations_offset(
        &self,
        unit: &DwarfUnit<R>,
        index: DebugLocListsIndex<R::Offset>,
    ) -> Result<LocationListsOffset<R::Offset>> {
        self.locations
            .get_offset(unit.encoding(), unit.loclists_base, index)
    }

    /// Iterate over the `LocationListEntry`s starting at the given offset.
    pub fn locations(
        &self,
        unit: &DwarfUnit<R>,
        offset: LocationListsOffset<R::Offset>,
    ) -> Result<LocListIter<R>> {
        self.locations.locations(
            offset,
            unit.encoding(),
            unit.low_pc,
            &self.debug_addr,
            unit.addr_base,
        )
    }

    /// Try to return an attribute value as a location list offset.
    ///
    /// If the attribute value is one of:
    ///
    /// - a `DW_FORM_sec_offset` reference to the `.debug_loc` or `.debug_loclists` sections
    /// - a `DW_FORM_loclistx` index into the `.debug_loclists` entries for the unit
    ///
    /// then return the location list offset of the location list.
    /// Returns `None` for other forms.
    pub fn attr_locations_offset(
        &self,
        unit: &DwarfUnit<R>,
        attr: AttributeValue<R, R::Offset>,
    ) -> Result<Option<LocationListsOffset<R::Offset>>> {
        match attr {
            AttributeValue::LocationListsRef(offset) => Ok(Some(offset)),
            AttributeValue::DebugLocListsIndex(index) => {
                self.locations_offset(unit, index).map(Some)
            }
            _ => Ok(None),
        }
    }

    /// Try to return an attribute value as a location list entry iterator.
    ///
    /// If the attribute value is one of:
    ///
    /// - a `DW_FORM_sec_offset` reference to the `.debug_loc` or `.debug_loclists` sections
    /// - a `DW_FORM_loclistx` index into the `.debug_loclists` entries for the unit
    ///
    /// then return an iterator over the entries in the location list.
    /// Returns `None` for other forms.
    pub fn attr_locations(
        &self,
        unit: &DwarfUnit<R>,
        attr: AttributeValue<R, R::Offset>,
    ) -> Result<Option<LocListIter<R>>> {
        match self.attr_locations_offset(unit, attr)? {
            Some(offset) => Ok(Some(self.locations(unit, offset)?)),
            None => Ok(None),
        }
    }
}

/// All of the commonly used information for a DWARF compilation unit.
#[derive(Debug)]
pub struct DwarfUnit<R: Reader> {
    /// The header of the unit.
    pub header: UnitHeader<R, R::Offset>,

    /// The parsed abbreviations for the unit.
    pub abbreviations: Abbreviations,

    /// The `DW_AT_name` attribute of the unit.
    pub name: Option<AttributeValue<R, R::Offset>>,

    /// The `DW_AT_comp_dir` attribute of the unit.
    pub comp_dir: Option<AttributeValue<R, R::Offset>>,

    /// The `DW_AT_low_pc` attribute of the unit. Defaults to 0.
    pub low_pc: u64,

    /// The `DW_AT_str_offsets_base` attribute of the unit. Defaults to 0.
    pub str_offsets_base: DebugStrOffsetsBase<R::Offset>,

    /// The `DW_AT_addr_base` attribute of the unit. Defaults to 0.
    pub addr_base: DebugAddrBase<R::Offset>,

    /// The `DW_AT_loclists_base` attribute of the unit. Defaults to 0.
    pub loclists_base: DebugLocListsBase<R::Offset>,

    /// The `DW_AT_rnglists_base` attribute of the unit. Defaults to 0.
    pub rnglists_base: DebugRngListsBase<R::Offset>,

    /// The line number program of the unit.
    pub line_program: Option<IncompleteLineProgram<R, R::Offset>>,
}

impl<R: Reader> DwarfUnit<R> {
    /// Construct a new `DwarfUnit` from the given header.
    pub fn new(dwarf: &Dwarf<R>, header: UnitHeader<R, R::Offset>) -> Result<Self> {
        let abbreviations = header.abbreviations(&dwarf.debug_abbrev)?;
        let mut name = None;
        let mut comp_dir = None;
        let mut low_pc = 0;
        // Defaults to 0 for GNU extensions.
        let mut str_offsets_base = DebugStrOffsetsBase(R::Offset::from_u8(0));
        let mut addr_base = DebugAddrBase(R::Offset::from_u8(0));
        let mut loclists_base = DebugLocListsBase(R::Offset::from_u8(0));
        let mut rnglists_base = DebugRngListsBase(R::Offset::from_u8(0));
        let mut line_program_offset = None;

        {
            let mut cursor = header.entries(&abbreviations);
            cursor.next_dfs()?;
            let root = cursor.current().ok_or(Error::MissingUnitDie)?;
            let mut attrs = root.attrs();
            while let Some(attr) = attrs.next()? {
                match attr.name() {
                    constants::DW_AT_name => {
                        name = Some(attr.value());
                    }
                    constants::DW_AT_comp_dir => {
                        comp_dir = Some(attr.value());
                    }
                    constants::DW_AT_low_pc => {
                        if let AttributeValue::Addr(address) = attr.value() {
                            low_pc = address;
                        }
                    }
                    constants::DW_AT_stmt_list => {
                        if let AttributeValue::DebugLineRef(offset) = attr.value() {
                            line_program_offset = Some(offset);
                        }
                    }
                    constants::DW_AT_str_offsets_base => {
                        if let AttributeValue::DebugStrOffsetsBase(base) = attr.value() {
                            str_offsets_base = base;
                        }
                    }
                    constants::DW_AT_addr_base => {
                        if let AttributeValue::DebugAddrBase(base) = attr.value() {
                            addr_base = base;
                        }
                    }
                    constants::DW_AT_loclists_base => {
                        if let AttributeValue::DebugLocListsBase(base) = attr.value() {
                            loclists_base = base;
                        }
                    }
                    constants::DW_AT_rnglists_base => {
                        if let AttributeValue::DebugRngListsBase(base) = attr.value() {
                            rnglists_base = base;
                        }
                    }
                    _ => {}
                }
            }
        }

        let line_program = match line_program_offset {
            Some(offset) => Some(dwarf.debug_line.program(
                offset,
                header.address_size(),
                comp_dir.clone(),
                name.clone(),
            )?),
            None => None,
        };

        Ok(DwarfUnit {
            header,
            abbreviations,
            name,
            comp_dir,
            low_pc,
            str_offsets_base,
            addr_base,
            loclists_base,
            rnglists_base,
            line_program,
        })
    }

    /// Return the encoding parameters for this unit.
    #[inline]
    pub fn encoding(&self) -> Encoding {
        self.header.encoding()
    }

    /// Navigate this unit's `DebuggingInformationEntry`s.
    #[inline]
    pub fn entries(&self) -> EntriesCursor<R> {
        self.header.entries(&self.abbreviations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use read::EndianSlice;
    use Endianity;

    /// Ensure that `Dwarf<R>` is covariant wrt R.
    #[test]
    fn test_dwarf_variance() {
        /// This only needs to compile.
        #[allow(dead_code)]
        fn f<'a: 'b, 'b, E: Endianity>(x: Dwarf<EndianSlice<'a, E>>) -> Dwarf<EndianSlice<'b, E>> {
            x
        }
    }
}