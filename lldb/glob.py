def summarize_glob(valobj, internal_dict):
    orig = valobj.GetChildMemberWithName("original")
    return "glob:" + orig.GetSummary()
