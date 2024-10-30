import re

def trim_namespace(name):
    m = re.match(".*::([^:]*)", name)
    if m:
        return m.group(1)
    return name

def summarize_one_arg_len(valobj, internal_dict):
    display_type_name = valobj.GetType().GetName()
    m = re.match(
        "([^<]+)<([^,]*),.*",
        display_type_name
    )

    return (trim_namespace(m.group(1)) + "<" +   
        trim_namespace(m.group(2)) +
        "> size=" + str(valobj.GetNumChildren()))

def summarize_two_arg_len(valobj, internal_dict):
    display_type_name = valobj.GetType().GetName()
    m = re.match(
        "([^<]+)<([^,]*),\s*([^,]*),.*",
        display_type_name
    )

    return (trim_namespace(m.group(1)) + "<" +   
        trim_namespace(m.group(2)) +
        ", " +
        trim_namespace(m.group(3)) +
        "> size=" + str(valobj.GetNumChildren()))