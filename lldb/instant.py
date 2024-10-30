import datetime

def summarize_std_unix_pal_instant(valobj, internal_dict):
    t       = valobj.GetChildMemberWithName("t")
    tv_sec  = t.GetChildMemberWithName("tv_sec").GetValueAsUnsigned()
    tv_nsec = t.GetChildMemberWithName("tv_nsec").GetChildMemberWithName("0").GetValueAsUnsigned()
    
    tv = tv_sec + float(tv_nsec)/1e9

    return "unix instant(" + str(tv) + ")"
