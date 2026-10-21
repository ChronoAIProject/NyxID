# Canonical package inventory, sorted by the caller with LC_ALL=C.
BEGIN { FS=":" }
/^P:/ { p=$2 } /^V:/ { v=$2 } /^A:/ { a=$2 }
/^$/ { if(p!="") print p "\t" v "\t" a; p="" }
END { if(p!="") print p "\t" v "\t" a }
