# Validate the entire catalog, then select complete requested roots. Kept in
# portable awk so minimal Debian/Ubuntu bases and local policy tests use the
# same parser. No network or package operation belongs in this file.
function fail(message) {
    print "APT snapshot catalog: " message > "/dev/stderr"
    bad = 1
    exit 1
}
BEGIN {
    FS = "\t"
    n = split(selectors, requested, " ")
    if (!n || selectors == "") fail("explicit roots are required")
    for (i = 1; i <= n; i++) {
        root = requested[i]
        if (root != "debian:bookworm" && root != "debian:trixie" &&
            root != "ubuntu:focal" && root != "ubuntu:noble") fail("unsupported root")
        if (wanted[root]++) fail("duplicate requested root")
    }
}
/^#/ { next }
{
    if (NF != 6 || length($6) != 64 || $6 ~ /[^0-9a-f]/) fail("malformed row or digest")
    root = $1 ":" $2
    if (root != "debian:bookworm" && root != "debian:trixie" &&
        root != "ubuntu:focal" && root != "ubuntu:noble") fail("unsupported catalog root")
    if ($3 != $2 && $3 != $2 "-updates" && $3 != $2 "-security") fail("unexpected suite")
    if (seen[root ":" $3]++) fail("duplicate suite")
    if ($1 == "debian") {
        if ($4 != "main") fail("unexpected Debian components")
        archive = ($3 == $2 "-security") ? "debian-security" : "debian"
        prefix = "https://snapshot.debian.org/archive/" archive "/"
    } else {
        if ($4 != "main,universe") fail("unexpected Ubuntu components")
        prefix = "https://snapshot.ubuntu.com/ubuntu/"
    }
    if (index($5, prefix) != 1) fail("unexpected snapshot origin")
    stamp = substr($5, length(prefix) + 1)
    if (length(stamp) != 16 || substr(stamp, 9, 1) != "T" || substr(stamp, 16) != "Z" ||
        substr(stamp, 1, 8) ~ /[^0-9]/ || substr(stamp, 10, 6) ~ /[^0-9]/)
        fail("invalid snapshot timestamp")
    if (timestamps[root] != "" && timestamps[root] != stamp) fail("mixed root timestamps")
    timestamps[root] = stamp
    count[root]++
    if (root in wanted) selected = selected $0 "\n"
}
END {
    if (bad) exit 1
    for (root in count) if (count[root] != 3) fail("incomplete catalog root")
    for (root in wanted) if (count[root] != 3) fail("requested root is missing or incomplete")
    printf "%s", selected
}
