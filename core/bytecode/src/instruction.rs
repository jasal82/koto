use std::fmt;

use koto_parser::MetaKeyId;

/// Decoded instructions produced by an [InstructionReader](crate::InstructionReader) for execution
/// in the runtime
///
/// For descriptions of each instruction's purpose, see corresponding [Op](crate::Op) entries.
#[allow(missing_docs)]
pub enum Instruction {
    Error {
        message: String,
    },
    Copy {
        target: u8,
        source: u8,
    },
    SetNull {
        register: u8,
    },
    SetBool {
        register: u8,
        value: bool,
    },
    SetNumber {
        register: u8,
        value: i64,
    },
    LoadFloat {
        register: u8,
        constant: u32,
    },
    LoadInt {
        register: u8,
        constant: u32,
    },
    LoadString {
        register: u8,
        constant: u32,
    },
    LoadNonLocal {
        register: u8,
        constant: u32,
    },
    ValueExport {
        name: u8,
        value: u8,
    },
    Import {
        register: u8,
    },
    MakeTempTuple {
        register: u8,
        start: u8,
        count: u8,
    },
    TempTupleToTuple {
        register: u8,
        source: u8,
    },
    MakeMap {
        register: u8,
        size_hint: u32,
    },
    SequenceStart {
        size_hint: u32,
    },
    SequencePush {
        value: u8,
    },
    SequencePushN {
        start: u8,
        count: u8,
    },
    SequenceToList {
        register: u8,
    },
    SequenceToTuple {
        register: u8,
    },
    Range {
        register: u8,
        start: u8,
        end: u8,
    },
    RangeInclusive {
        register: u8,
        start: u8,
        end: u8,
    },
    RangeTo {
        register: u8,
        end: u8,
    },
    RangeToInclusive {
        register: u8,
        end: u8,
    },
    RangeFrom {
        register: u8,
        start: u8,
    },
    RangeFull {
        register: u8,
    },
    MakeIterator {
        register: u8,
        iterable: u8,
    },
    Function {
        register: u8,
        arg_count: u8,
        capture_count: u8,
        variadic: bool,
        generator: bool,
        arg_is_unpacked_tuple: bool,
        size: u16,
    },
    Capture {
        function: u8,
        target: u8,
        source: u8,
    },
    Negate {
        register: u8,
        value: u8,
    },
    Not {
        register: u8,
        value: u8,
    },
    Add {
        register: u8,
        lhs: u8,
        rhs: u8,
    },
    Subtract {
        register: u8,
        lhs: u8,
        rhs: u8,
    },
    Multiply {
        register: u8,
        lhs: u8,
        rhs: u8,
    },
    Divide {
        register: u8,
        lhs: u8,
        rhs: u8,
    },
    Remainder {
        register: u8,
        lhs: u8,
        rhs: u8,
    },
    AddAssign {
        lhs: u8,
        rhs: u8,
    },
    SubtractAssign {
        lhs: u8,
        rhs: u8,
    },
    MultiplyAssign {
        lhs: u8,
        rhs: u8,
    },
    DivideAssign {
        lhs: u8,
        rhs: u8,
    },
    RemainderAssign {
        lhs: u8,
        rhs: u8,
    },
    Less {
        register: u8,
        lhs: u8,
        rhs: u8,
    },
    LessOrEqual {
        register: u8,
        lhs: u8,
        rhs: u8,
    },
    Greater {
        register: u8,
        lhs: u8,
        rhs: u8,
    },
    GreaterOrEqual {
        register: u8,
        lhs: u8,
        rhs: u8,
    },
    Equal {
        register: u8,
        lhs: u8,
        rhs: u8,
    },
    NotEqual {
        register: u8,
        lhs: u8,
        rhs: u8,
    },
    Jump {
        offset: u16,
    },
    JumpBack {
        offset: u16,
    },
    JumpIfTrue {
        register: u8,
        offset: u16,
    },
    JumpIfFalse {
        register: u8,
        offset: u16,
    },
    Call {
        result: u8,
        function: u8,
        frame_base: u8,
        arg_count: u8,
    },
    CallInstance {
        result: u8,
        function: u8,
        frame_base: u8,
        arg_count: u8,
        instance: u8,
    },
    Return {
        register: u8,
    },
    Yield {
        register: u8,
    },
    Throw {
        register: u8,
    },
    Size {
        register: u8,
        value: u8,
    },
    IterNext {
        result: Option<u8>,
        iterator: u8,
        jump_offset: u16,
        temporary_output: bool,
    },
    TempIndex {
        register: u8,
        value: u8,
        index: i8,
    },
    SliceFrom {
        register: u8,
        value: u8,
        index: i8,
    },
    SliceTo {
        register: u8,
        value: u8,
        index: i8,
    },
    IsTuple {
        register: u8,
        value: u8,
    },
    IsList {
        register: u8,
        value: u8,
    },
    Index {
        register: u8,
        value: u8,
        index: u8,
    },
    SetIndex {
        register: u8,
        index: u8,
        value: u8,
    },
    MapInsert {
        register: u8,
        key: u8,
        value: u8,
    },
    MetaInsert {
        register: u8,
        value: u8,
        id: MetaKeyId,
    },
    MetaInsertNamed {
        register: u8,
        value: u8,
        id: MetaKeyId,
        name: u8,
    },
    MetaExport {
        id: MetaKeyId,
        value: u8,
    },
    MetaExportNamed {
        id: MetaKeyId,
        name: u8,
        value: u8,
    },
    Access {
        register: u8,
        value: u8,
        key: u32,
    },
    AccessString {
        register: u8,
        value: u8,
        key: u8,
    },
    TryStart {
        arg_register: u8,
        catch_offset: u16,
    },
    TryEnd,
    Debug {
        register: u8,
        constant: u32,
    },
    CheckType {
        register: u8,
        type_id: TypeId,
    },
    CheckSizeEqual {
        register: u8,
        size: usize,
    },
    CheckSizeMin {
        register: u8,
        size: usize,
    },
    StringStart {
        size_hint: u32,
    },
    StringPush {
        value: u8,
    },
    StringFinish {
        register: u8,
    },
}

#[derive(Debug)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum TypeId {
    List,
    Tuple,
}

impl TypeId {
    /// Produces a [TypeId] from the given byte
    pub fn from_byte(byte: u8) -> Result<Self, u8> {
        if byte == Self::List as u8 {
            Ok(Self::List)
        } else if byte == Self::Tuple as u8 {
            Ok(Self::Tuple)
        } else {
            Err(byte)
        }
    }
}

/// Flags used to define the properties of a Function
pub struct FunctionFlags {
    /// True if the function has a variadic argument
    pub variadic: bool,
    /// True if the function is a generator
    pub generator: bool,
    /// True if the function has a single argument which is an unpacked tuple
    pub arg_is_unpacked_tuple: bool,
}

impl FunctionFlags {
    /// Corresponding to [FunctionFlags::variadic]
    pub const VARIADIC: u8 = 1 << 0;
    /// Corresponding to [FunctionFlags::generator]
    pub const GENERATOR: u8 = 1 << 1;
    /// Corresponding to [FunctionFlags::arg_is_unpacked_tuple]
    pub const ARG_IS_UNPACKED_TUPLE: u8 = 1 << 2;

    /// Initializes a flags struct from a byte
    pub fn from_byte(byte: u8) -> Self {
        Self {
            variadic: byte & Self::VARIADIC != 0,
            generator: byte & Self::GENERATOR != 0,
            arg_is_unpacked_tuple: byte & Self::ARG_IS_UNPACKED_TUPLE != 0,
        }
    }

    /// Returns a byte containing the packed flags
    pub fn as_byte(&self) -> u8 {
        let mut result = 0;
        if self.variadic {
            result |= Self::VARIADIC;
        }
        if self.generator {
            result |= Self::GENERATOR;
        }
        if self.arg_is_unpacked_tuple {
            result |= Self::ARG_IS_UNPACKED_TUPLE;
        }
        result
    }
}

impl fmt::Debug for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Instruction::*;
        match self {
            Error { message } => unreachable!("{message}"),
            Copy { target, source } => write!(f, "Copy\t\tresult: {target}\tsource: {source}"),
            SetNull { register } => write!(f, "SetNull\t\tresult: {register}"),
            SetBool { register, value } => {
                write!(f, "SetBool\t\tresult: {register}\tvalue: {value}")
            }
            SetNumber { register, value } => {
                write!(f, "SetNumber\tresult: {register}\tvalue: {value}")
            }
            LoadFloat { register, constant } => {
                write!(f, "LoadFloat\tresult: {register}\tconstant: {constant}")
            }
            LoadInt { register, constant } => {
                write!(f, "LoadInt\t\tresult: {register}\tconstant: {constant}")
            }
            LoadString { register, constant } => {
                write!(f, "LoadString\tresult: {register}\tconstant: {constant}")
            }
            LoadNonLocal { register, constant } => {
                write!(f, "LoadNonLocal\tresult: {register}\tconstant: {constant}")
            }
            ValueExport { name, value } => {
                write!(f, "ValueExport\tname: {name}\t\tvalue: {value}")
            }
            Import { register } => write!(f, "Import\t\tregister: {register}"),
            MakeTempTuple {
                register,
                start,
                count,
            } => write!(
                f,
                "MakeTempTuple\tresult: {register}\tstart: {start}\tcount: {count}"
            ),
            TempTupleToTuple { register, source } => {
                write!(f, "TempTupleToTuple\tresult: {register}\tsource: {source}")
            }
            MakeMap {
                register,
                size_hint,
            } => write!(f, "MakeMap\t\tresult: {register}\tsize_hint: {size_hint}"),
            SequenceStart { size_hint } => write!(f, "SequenceStart\tsize_hint: {size_hint}"),
            SequencePush { value } => {
                write!(f, "SequencePush\tvalue: {value}")
            }
            SequencePushN { start, count } => {
                write!(f, "SequencePushN\tstart: {start}\tcount: {count}",)
            }
            SequenceToList { register } => write!(f, "SequenceToList\tregister: {register}"),
            SequenceToTuple { register } => write!(f, "SequenceToTuple\tregister: {register}"),
            Range {
                register,
                start,
                end,
            } => write!(f, "Range\t\tresult: {register}\tstart: {start}\tend: {end}",),
            RangeInclusive {
                register,
                start,
                end,
            } => write!(
                f,
                "RangeInclusive\tresult: {register}\tstart: {start}\tend: {end}",
            ),
            RangeTo { register, end } => write!(f, "RangeTo\t\tresult: {register}\tend: {end}"),
            RangeToInclusive { register, end } => {
                write!(f, "RangeToIncl\tresult: {register}\tend: {end}")
            }
            RangeFrom { register, start } => {
                write!(f, "RangeFrom\tresult: {register}\tstart: {start}")
            }
            RangeFull { register } => write!(f, "RangeFull\tresult: {register}"),
            MakeIterator { register, iterable } => {
                write!(f, "MakeIterator\tresult: {register}\titerable: {iterable}",)
            }
            Function {
                register,
                arg_count,
                capture_count,
                variadic,
                generator,
                arg_is_unpacked_tuple,
                size,
            } => write!(
                f,
                "Function\tresult: {register}\targs: {arg_count}\
                 \t\tcaptures: {capture_count}
                 \t\t\tsize: {size} \tgenerator: {generator}
                 \t\t\tvariadic: {variadic}\targ_is_unpacked_tuple: {arg_is_unpacked_tuple}",
            ),
            Capture {
                function,
                target,
                source,
            } => write!(
                f,
                "Capture\t\tfunction: {function}\ttarget: {target}\tsource: {source}",
            ),
            Negate { register, value } => {
                write!(f, "Negate\t\tresult: {register}\tsource: {value}")
            }
            Not { register, value } => {
                write!(f, "Not\t\tresult: {register}\tsource: {value}")
            }
            Add { register, lhs, rhs } => {
                write!(f, "Add\t\tresult: {register}\tlhs: {lhs}\t\trhs: {rhs}")
            }
            Subtract { register, lhs, rhs } => {
                write!(f, "Subtract\tresult: {register}\tlhs: {lhs}\t\trhs: {rhs}")
            }
            Multiply { register, lhs, rhs } => {
                write!(f, "Multiply\tresult: {register}\tlhs: {lhs}\t\trhs: {rhs}")
            }
            Divide { register, lhs, rhs } => {
                write!(f, "Divide\t\tresult: {register}\tlhs: {lhs}\t\trhs: {rhs}")
            }
            Remainder { register, lhs, rhs } => {
                write!(f, "Remainder\tresult: {register}\tlhs: {lhs}\t\trhs: {rhs}")
            }
            AddAssign { lhs, rhs } => {
                write!(f, "AddAssign\tlhs: {lhs}\t\trhs: {rhs}")
            }
            SubtractAssign { lhs, rhs } => {
                write!(f, "SubAssign\tlhs: {lhs}\t\trhs: {rhs}")
            }
            MultiplyAssign { lhs, rhs } => {
                write!(f, "MulAssign\tlhs: {lhs}\t\trhs: {rhs}")
            }
            DivideAssign { lhs, rhs } => {
                write!(f, "DivAssign\tlhs: {lhs}\t\trhs: {rhs}")
            }
            RemainderAssign { lhs, rhs } => {
                write!(f, "RemAssign\tlhs: {lhs}\t\trhs: {rhs}")
            }
            Less { register, lhs, rhs } => {
                write!(f, "Less\t\tresult: {register}\tlhs: {lhs}\t\trhs: {rhs}")
            }
            LessOrEqual { register, lhs, rhs } => write!(
                f,
                "LessOrEqual\tresult: {register}\tlhs: {lhs}\t\trhs: {rhs}",
            ),
            Greater { register, lhs, rhs } => {
                write!(f, "Greater\t\tresult: {register}\tlhs: {lhs}\t\trhs: {rhs}")
            }
            GreaterOrEqual { register, lhs, rhs } => write!(
                f,
                "GreaterOrEqual\tresult: {register}\tlhs: {lhs}\t\trhs: {rhs}",
            ),
            Equal { register, lhs, rhs } => {
                write!(f, "Equal\t\tresult: {register}\tlhs: {lhs}\t\trhs: {rhs}")
            }
            NotEqual { register, lhs, rhs } => {
                write!(f, "NotEqual\tresult: {register}\tlhs: {lhs}\t\trhs: {rhs}")
            }
            Jump { offset } => write!(f, "Jump\t\toffset: {offset}"),
            JumpBack { offset } => write!(f, "JumpBack\toffset: {offset}"),
            JumpIfTrue { register, offset } => {
                write!(f, "JumpIfTrue\tresult: {register}\toffset: {offset}")
            }
            JumpIfFalse { register, offset } => {
                write!(f, "JumpIfFalse\tresult: {register}\toffset: {offset}")
            }
            Call {
                result,
                function,
                frame_base,
                arg_count,
            } => write!(
                f,
                "Call\t\tresult: {result}\tfunction: {function}\t\
                 frame base: {frame_base}\targs: {arg_count}",
            ),
            CallInstance {
                result,
                function,
                frame_base,
                arg_count,
                instance,
            } => write!(
                f,
                "CallInstance\tresult: {result}\tfunction: {function}\tframe_base: {frame_base}
                 \t\t\targs: {arg_count}\t\tinstance: {instance}",
            ),
            Return { register } => write!(f, "Return\t\tresult: {register}"),
            Yield { register } => write!(f, "Yield\t\tresult: {register}"),
            Throw { register } => write!(f, "Throw\t\tresult: {register}"),
            Size { register, value } => write!(f, "Size\t\tresult: {register}\tvalue: {value}"),
            IterNext {
                result,
                iterator,
                jump_offset,
                temporary_output,
            } => write!(
                f,
                "IterNext\t{}iterator: {iterator}\t\
                jump: {jump_offset} \ttemp: {temporary_output}",
                result.map_or(String::new(), |result| format!("result: {result}\t")),
            ),
            TempIndex {
                register,
                value,
                index,
            } => write!(
                f,
                "TempIndex\tresult: {register}\tvalue: {value}\tindex: {index}",
            ),
            SliceFrom {
                register,
                value,
                index,
            } => write!(
                f,
                "SliceFrom\tresult: {register}\tvalue: {value}\tindex: {index}",
            ),
            SliceTo {
                register,
                value,
                index,
            } => write!(
                f,
                "SliceTo\t\tresult: {register}\tvalue: {value}\tindex: {index}"
            ),
            IsTuple { register, value } => {
                write!(f, "IsTuple\t\tresult: {register}\tvalue: {value}")
            }
            IsList { register, value } => {
                write!(f, "IsList\t\tresult: {register}\tvalue: {value}")
            }
            Index {
                register,
                value,
                index,
            } => write!(
                f,
                "Index\t\tresult: {register}\tvalue: {value}\tindex: {index}"
            ),
            SetIndex {
                register,
                index,
                value,
            } => write!(
                f,
                "SetIndex\tregister: {register}\tindex: {index}\tvalue: {value}"
            ),
            MapInsert {
                register,
                value,
                key,
            } => write!(
                f,
                "MapInsert\tmap: {register}\t\tvalue: {value}\tkey: {key}"
            ),
            MetaInsert {
                register,
                value,
                id,
            } => write!(
                f,
                "MetaInsert\tmap: {register}\t\tid: {id:?}\tvalue: {value}",
            ),
            MetaInsertNamed {
                register,
                id,
                name,
                value,
            } => write!(
                f,
                "MetaInsertNamed\tmap: {register}\t\tid: {id:?}\tname: {name}\t\tvalue: {value}",
            ),
            MetaExport { id, value } => write!(f, "MetaExport\tid: {id:?}\tvalue: {value}"),
            MetaExportNamed { id, name, value } => write!(
                f,
                "MetaExportNamed\tid: {id:?}\tname: {name}\tvalue: {value}",
            ),
            Access {
                register,
                value,
                key,
            } => write!(
                f,
                "Access\t\tresult: {register}\tvalue: {value}\tkey: {key}"
            ),
            AccessString {
                register,
                value,
                key,
            } => write!(
                f,
                "AccessString\tresult: {register}\tvalue: {value}\tkey: {key}"
            ),
            TryStart {
                arg_register,
                catch_offset,
            } => write!(
                f,
                "TryStart\targ register: {arg_register}\tcatch offset: {catch_offset}",
            ),
            TryEnd => write!(f, "TryEnd"),
            Debug { register, constant } => {
                write!(f, "Debug\t\tregister: {register}\tconstant: {constant}")
            }
            CheckType { register, type_id } => {
                write!(f, "CheckType\tregister: {register}\ttype: {type_id:?}")
            }
            CheckSizeEqual { register, size } => {
                write!(f, "CheckSizeEqual\tregister: {register}\tsize: {size}")
            }
            CheckSizeMin { register, size } => {
                write!(f, "CheckSizeMin\tregister: {register}\tsize: {size}")
            }
            StringStart { size_hint } => {
                write!(f, "StringStart\tsize hint: {size_hint}")
            }
            StringPush { value } => {
                write!(f, "StringPush\tvalue: {value}")
            }
            StringFinish { register } => {
                write!(f, "StringFinish\tregister: {register}")
            }
        }
    }
}
