
class StdVecSyntheticProvider:
    """Pretty-printer for alloc::vec::Vec<T>

    struct Vec<T> { buf: RawVec<T>, len: usize }
    rust 1.75: struct RawVec<T> { ptr: Unique<T>, cap: usize, ... }
    rust 1.76: struct RawVec<T> { ptr: Unique<T>, cap: Cap(usize), ... }
    rust 1.31.1: struct Unique<T: ?Sized> { pointer: NonZero<*const T>, ... }
    rust 1.33.0: struct Unique<T: ?Sized> { pointer: *const T, ... }
    rust 1.62.0: struct Unique<T: ?Sized> { pointer: NonNull<T>, ... }
    struct NonZero<T>(T)
    struct NonNull<T> { pointer: *const T }
    """

    def __init__(self, valobj, dict):
        # type: (SBValue, dict) -> StdVecSyntheticProvider
        # logger = Logger.Logger()
        # logger >> "[StdVecSyntheticProvider] for " + str(valobj.GetName())
        self.valobj = valobj
        self.update()

    def num_children(self):
        # type: () -> int
        return self.length

    def get_child_index(self, name):
        # type: (str) -> int
        index = name.lstrip('[').rstrip(']')
        if index.isdigit():
            return int(index)
        else:
            return -1

    def get_child_at_index(self, index):
        # type: (int) -> SBValue
        start = self.data_ptr.GetValueAsUnsigned()
        address = start + index * self.element_type_size
        element = self.data_ptr.CreateValueFromAddress("[%s]" % index, address, self.element_type)
        return element

    def update(self):
        # type: () -> None
        self.length = self.valobj.GetChildMemberWithName("len").GetValueAsUnsigned()
        self.buf = self.valobj.GetChildMemberWithName("buf").GetChildMemberWithName("inner")

        self.data_ptr = unwrap_unique_or_non_null(self.buf.GetChildMemberWithName("ptr"))

        self.element_type = self.valobj.GetType().GetTemplateArgumentType(0)
        self.element_type_size = self.element_type.GetByteSize()

    def has_children(self):
        # type: () -> bool
        return True

def is_std_vec(type_obj, _dict):
    # type: (SBValue) -> bool
    to_ret = type_obj.GetName().startswith("alloc::vec::Vec<")
    # if to_ret:
    #     print("is_std_vec(%s) = %s" % (type_name, to_ret))
    return to_ret

def unwrap_unique_or_non_null(unique_or_nonnull):
    # BACKCOMPAT: rust 1.32
    # https://github.com/rust-lang/rust/commit/7a0911528058e87d22ea305695f4047572c5e067
    # BACKCOMPAT: rust 1.60
    # https://github.com/rust-lang/rust/commit/2a91eeac1a2d27dd3de1bf55515d765da20fd86f
    ptr = unique_or_nonnull.GetChildMemberWithName("pointer")
    return ptr if ptr.TypeIsPointerType() else ptr.GetChildAtIndex(0)
