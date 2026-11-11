#!/bin/sh
# CHANGELOG.md al publicar una versión (lo usa .github/workflows/release.yml). POSIX sh y awk: anda con el awk
# de Ubuntu (mawk o gawk) y con el de macOS.
#
#   sh scripts/changelog.sh publicar <versión> <AAAA-MM-DD> [archivo]
#       Si «## Sin publicar» tiene algo, pasa a ser «## <versión> — <fecha>» y deja arriba un «## Sin publicar»
#       vacío. No hace nada si la sección está vacía o si ya hay una sección de esa versión.
#   sh scripts/changelog.sh notas <versión> [archivo]
#       Escribe el texto de la sección de esa versión (sin el título ni las líneas vacías de los bordes); nada si
#       no está o está vacía.
#
# Las pruebas están en scripts/changelog-test.sh.
set -eu

uso() {
    echo "uso: $0 publicar <versión> <AAAA-MM-DD> [archivo] | notas <versión> [archivo]" >&2
    exit 2
}

[ $# -ge 2 ] || uso
cmd=$1
version=$2

case "$cmd" in
publicar)
    [ $# -ge 3 ] || uso
    fecha=$3
    file=${4:-CHANGELOG.md}
    [ -f "$file" ] || { echo "No existe $file" >&2; exit 1; }
    # Qué hay: "<tiene algo sin publicar> <ya está la versión>".
    estado=$(awk -v v="$version" '
        { l = $0; sub(/\r$/, "", l) }
        l == "## Sin publicar" { dentro = 1; next }
        index(l, "## ") == 1 { dentro = 0; if (index(l, "## " v " ") == 1 || l == "## " v) ya = 1; next }
        dentro && l ~ /[^ \t]/ { algo = 1 }
        END { print (algo ? 1 : 0) " " (ya ? 1 : 0) }
    ' "$file")
    case "$estado" in
    "0 "*) echo "«Sin publicar» está vacía: CHANGELOG.md queda igual"; exit 0 ;;
    *" 1") echo "Ya hay una sección de la ${version}: CHANGELOG.md queda igual"; exit 0 ;;
    esac
    tmp="$file.tmp.$$"
    awk -v v="$version" -v d="$fecha" '
        {
            cr = ""
            if ($0 ~ /\r$/) cr = "\r"
            l = $0; sub(/\r$/, "", l)
        }
        !hecho && l == "## Sin publicar" {
            print "## Sin publicar" cr
            print cr
            print "## " v " — " d cr
            hecho = 1
            next
        }
        { print }
    ' "$file" > "$tmp"
    mv "$tmp" "$file"
    echo "«Sin publicar» pasó a ser «${version} — ${fecha}»"
    ;;
notas)
    file=${3:-CHANGELOG.md}
    [ -f "$file" ] || exit 0
    awk -v v="$version" '
        { l = $0; sub(/\r$/, "", l); sub(/^\357\273\277/, "", l) }
        index(l, "## ") == 1 {
            if (dentro) exit
            if (index(l, "## " v " ") == 1 || l == "## " v) dentro = 1
            next
        }
        dentro {
            if (l ~ /^[ \t]*$/) { vacias++; next }
            if (hay) for (i = 0; i < vacias; i++) print ""
            vacias = 0
            hay = 1
            print l
        }
    ' "$file"
    ;;
*)
    uso
    ;;
esac
