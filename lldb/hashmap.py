from lldb import SBData, SBError, eBasicTypeLong, eBasicTypeUnsignedLong, eBasicTypeUnsignedChar

class StdHashMapSyntheticProvider:
    """Pretty-printer for hashbrown's HashMap"""

    def __init__(self, valobj, dict, show_values=True):
        # type: (SBValue, dict, bool) -> StdHashMapSyntheticProvider
        self.valobj = valobj
        self.show_values = show_values
        self.update()

    def num_children(self):
        # type: () -> int
        return self.size

    def get_child_index(self, name):
        # type: (str) -> int
        index = name.lstrip('[').rstrip(']')
        if index.isdigit():
            return int(index)
        else:
            return -1

    def get_child_at_index(self, index):
        # type: (int) -> SBValue
        pairs_start = self.data_ptr.GetValueAsUnsigned()
        idx = self.valid_indices[index]
        if self.new_layout:
            idx = -(idx + 1)
        address = pairs_start + idx * self.pair_type_size
        element = self.data_ptr.CreateValueFromAddress("[%s]" % index, address, self.pair_type)
        if self.show_values:
            return element
        else:
            key = element.GetChildAtIndex(0)
            return self.valobj.CreateValueFromData("[%s]" % index, key.GetData(), key.GetType())

    def update(self):
        # type: () -> None
        table = self.table()
        inner_table = table.GetChildMemberWithName("table")

        capacity = inner_table.GetChildMemberWithName("bucket_mask").GetValueAsUnsigned() + 1
        ctrl = inner_table.GetChildMemberWithName("ctrl").GetChildAtIndex(0)

        self.size = inner_table.GetChildMemberWithName("items").GetValueAsUnsigned()
        self.pair_type = table.type.template_args[0]
        if self.pair_type.IsTypedefType():
            self.pair_type = self.pair_type.GetTypedefedType()
        self.pair_type_size = self.pair_type.GetByteSize()

        self.new_layout = not inner_table.GetChildMemberWithName("data").IsValid()
        if self.new_layout:
            self.data_ptr = ctrl.Cast(self.pair_type.GetPointerType())
        else:
            self.data_ptr = inner_table.GetChildMemberWithName("data").GetChildAtIndex(0)

        u8_type = self.valobj.GetTarget().GetBasicType(eBasicTypeUnsignedChar)
        u8_type_size = self.valobj.GetTarget().GetBasicType(eBasicTypeUnsignedChar).GetByteSize()

        self.valid_indices = []
        for idx in range(capacity):
            address = ctrl.GetValueAsUnsigned() + idx * u8_type_size
            value = ctrl.CreateValueFromAddress("ctrl[%s]" % idx, address,
                                                u8_type).GetValueAsUnsigned()
            is_present = value & 128 == 0
            if is_present:
                self.valid_indices.append(idx)

    def table(self):
        hashbrown_hashmap = self.valobj.GetChildMemberWithName("base")
        return hashbrown_hashmap.GetChildMemberWithName("table")

    def has_children(self):
        # type: () -> bool
        return True

def is_std_hashmap(type_obj, dict):
    # type: (SBValue, dict) -> bool
    return type_obj.GetName().startswith("std::collections::hash::map::HashMap<")

class StdHashSetSyntheticProvider(StdHashMapSyntheticProvider):
    def __init__(self, valobj, dict):
        # type: (SBValue, dict) -> StdHashSetSyntheticProvider
        super(StdHashSetSyntheticProvider, self).__init__(valobj, dict, show_values=False)
    
    def table(self):
        hashbrown_hashset = self.valobj.GetChildMemberWithName("base")
        hashbrown_hashmap = hashbrown_hashset.GetChildMemberWithName("map")
        return hashbrown_hashmap.GetChildMemberWithName("table")


def is_std_hashset(type_obj, dict):
    # type: (SBValue, dict) -> bool
    return type_obj.GetName().startswith("std::collections::hash::set::HashSet<")