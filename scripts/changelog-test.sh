#!/bin/sh
# Pruebas de scripts/changelog.sh contra CHANGELOG de ejemplo: sh scripts/changelog-test.sh
set -eu

dir=$(cd "$(dirname "$0")" && pwd)
cl="$dir/changelog.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fallas=0

igual() { # nombre, esperado, obtenido (archivos)
    if cmp -s "$2" "$3"; then
        echo "ok    $1"
    else
        echo "FALLA $1"
        diff "$2" "$3" || true
        fallas=$((fallas + 1))
    fi
}

cat > "$tmp/ejemplo.md" <<'EOF'
# Novedades de xdir

## Sin publicar
### Nuevo
- **Algo** nuevo con `código`.
  - Un detalle.

### Arreglos
- Un arreglo.

## 0.1.23 — 2026-11-08
- Lo de antes.

## 0.1.2 — 2026-10-26
- Lo de mucho antes.
EOF

# publicar: la sección pasa a ser la versión y queda una vacía arriba.
cp "$tmp/ejemplo.md" "$tmp/a.md"
sh "$cl" publicar 0.1.24 2026-11-10 "$tmp/a.md" > /dev/null
cat > "$tmp/esperado.md" <<'EOF'
# Novedades de xdir

## Sin publicar

## 0.1.24 — 2026-11-10
### Nuevo
- **Algo** nuevo con `código`.
  - Un detalle.

### Arreglos
- Un arreglo.

## 0.1.23 — 2026-11-08
- Lo de antes.

## 0.1.2 — 2026-10-26
- Lo de mucho antes.
EOF
igual "publicar renombra la sección" "$tmp/esperado.md" "$tmp/a.md"

# notas: el texto de la versión, sin título ni líneas vacías en los bordes.
sh "$cl" notas 0.1.24 "$tmp/a.md" > "$tmp/notas.md"
cat > "$tmp/esperado.md" <<'EOF'
### Nuevo
- **Algo** nuevo con `código`.
  - Un detalle.

### Arreglos
- Un arreglo.
EOF
igual "notas de la versión publicada" "$tmp/esperado.md" "$tmp/notas.md"

# Publicar dos veces la misma versión no hace nada (la sección vacía queda vacía).
cp "$tmp/a.md" "$tmp/b.md"
sh "$cl" publicar 0.1.24 2026-11-10 "$tmp/b.md" > /dev/null
igual "publicar con «Sin publicar» vacía no cambia nada" "$tmp/a.md" "$tmp/b.md"

# Una versión que ya tiene sección no se duplica aunque haya algo sin publicar.
cp "$tmp/ejemplo.md" "$tmp/c.md"
sh "$cl" publicar 0.1.23 2026-11-10 "$tmp/c.md" > /dev/null
igual "publicar una versión que ya está no cambia nada" "$tmp/ejemplo.md" "$tmp/c.md"

# notas de una versión que no está: nada (el release usa --generate-notes).
sh "$cl" notas 9.9.9 "$tmp/ejemplo.md" > "$tmp/notas.md"
: > "$tmp/vacio.md"
igual "notas de una versión que no está" "$tmp/vacio.md" "$tmp/notas.md"

# notas de 0.1.2 no se confunde con 0.1.23, y la última sección termina en el fin del archivo.
sh "$cl" notas 0.1.2 "$tmp/ejemplo.md" > "$tmp/notas.md"
printf '%s\n' '- Lo de mucho antes.' > "$tmp/esperado.md"
igual "notas de 0.1.2 (prefijo de 0.1.23)" "$tmp/esperado.md" "$tmp/notas.md"
sh "$cl" notas 0.1.23 "$tmp/ejemplo.md" > "$tmp/notas.md"
printf '%s\n' '- Lo de antes.' > "$tmp/esperado.md"
igual "notas de 0.1.23" "$tmp/esperado.md" "$tmp/notas.md"

# Sin publicar vacía de entrada: no se renombra y notas da vacío.
printf '# Novedades de xdir\n\n## Sin publicar\n\n## 0.1.23 — 2026-11-08\n- Lo de antes.\n' > "$tmp/d.md"
cp "$tmp/d.md" "$tmp/d0.md"
sh "$cl" publicar 0.1.24 2026-11-10 "$tmp/d.md" > /dev/null
igual "«Sin publicar» vacía queda igual" "$tmp/d0.md" "$tmp/d.md"
sh "$cl" notas 0.1.24 "$tmp/d.md" > "$tmp/notas.md"
igual "notas de una versión sin sección" "$tmp/vacio.md" "$tmp/notas.md"

# CRLF: se reconoce y los renglones nuevos llevan CRLF.
printf '# Novedades\r\n\r\n## Sin publicar\r\n- Uno.\r\n' > "$tmp/e.md"
sh "$cl" publicar 1.0.0 2027-01-02 "$tmp/e.md" > /dev/null
printf '# Novedades\r\n\r\n## Sin publicar\r\n\r\n## 1.0.0 — 2027-01-02\r\n- Uno.\r\n' > "$tmp/esperado.md"
igual "CRLF" "$tmp/esperado.md" "$tmp/e.md"
sh "$cl" notas 1.0.0 "$tmp/e.md" > "$tmp/notas.md"
printf '%s\n' '- Uno.' > "$tmp/esperado.md"
igual "notas de un archivo CRLF, sin el retorno de carro" "$tmp/esperado.md" "$tmp/notas.md"

# El CHANGELOG del repo se puede publicar: queda «## Sin publicar» arriba de todo y, si tenía algo, la versión.
cp "$dir/../CHANGELOG.md" "$tmp/repo.md"
if sh "$cl" publicar 0.0.0-prueba 2026-01-01 "$tmp/repo.md" > /dev/null && grep -q '^## Sin publicar$' "$tmp/repo.md" &&
    [ "$(grep '^## ' "$tmp/repo.md" | head -1)" = "## Sin publicar" ]; then
    echo "ok    el CHANGELOG del repo se publica"
else
    echo "FALLA el CHANGELOG del repo se publica"
    fallas=$((fallas + 1))
fi

if [ "$fallas" -ne 0 ]; then
    echo "$fallas pruebas fallaron"
    exit 1
fi
echo "todas las pruebas pasaron"
