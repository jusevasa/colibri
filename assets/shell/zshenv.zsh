# Integración de Colibrí con zsh.
#
# Colibrí arranca zsh con ZDOTDIR apuntando a la carpeta de este archivo, así que zsh lo
# lee como si fuera tu .zshenv. Lo primero que hace es devolver ZDOTDIR a su valor y
# cargar tu .zshenv de verdad: el resto del arranque (.zprofile, .zshrc, .zlogin) es el
# tuyo, sin tocar.
#
# Lo único que añade es un gancho `precmd` que envuelve el prompt en un hiperenlace
# (OSC 8) «colibri-prompt:<carpeta>». No se ve, pero las celdas del prompt quedan
# marcadas en la terminal: Colibrí sabe así qué celdas son prompt y en qué carpeta se
# pintó, sin adivinarlo por el texto. En otra terminal esto no se carga.

if [[ -n ${COLIBRI_ZDOTDIR+x} ]]; then
  export ZDOTDIR=$COLIBRI_ZDOTDIR
  unset COLIBRI_ZDOTDIR
else
  unset ZDOTDIR
fi
if [[ -r ${ZDOTDIR:-$HOME}/.zshenv ]]; then
  builtin source -- ${ZDOTDIR:-$HOME}/.zshenv
fi

if [[ -o interactive ]]; then
  # Deja en REPLY el texto codificado byte a byte (%XX), con el % doblado porque va
  # dentro de PROMPT. Así ningún carácter de la ruta puede cortar la secuencia ni
  # expandirse como parte del prompt.
  _colibri_encode() {
    emulate -L zsh
    unsetopt multibyte
    local s=$1 c
    local -i i
    REPLY=
    for (( i = 1; i <= $#s; i++ )); do
      c=$s[i]
      if [[ $c == [A-Za-z0-9/._~-] ]]; then
        REPLY+=$c
      else
        REPLY+=%%${(l:2::0:)$(( [##16] #c ))}
      fi
    done
  }

  # Envuelve PROMPT con la marca de la carpeta actual.
  _colibri_mark() {
    emulate -L zsh
    local open=$'%{\e]8;;colibri-prompt:' close=$'%{\e]8;;\a%}' REPLY
    # Quita la marca del prompt anterior, si el tema no ha vuelto a escribir PROMPT.
    if [[ $PROMPT == ${open}* ]]; then
      PROMPT=${PROMPT#${open}*'%}'}
      PROMPT=${PROMPT%${close}}
    fi
    if [[ -n $PROMPT ]]; then
      # La carpeta solo se vuelve a codificar cuando cambia.
      if [[ $PWD != ${_colibri_pwd-} ]]; then
        _colibri_encode $PWD
        typeset -g _colibri_pwd=$PWD _colibri_enc=$REPLY
      fi
      # Tras la carpeta, cómo acabó el comando anterior: «;s=<código>;t=<milisegundos>».
      PROMPT=${open}${_colibri_enc}${_colibri_result-}$'\a%}'${PROMPT}${close}
    fi
  }

  # Anota cuándo empieza cada comando, para saber cuánto duró.
  _colibri_preexec() {
    typeset -gF _colibri_t0=$EPOCHREALTIME
  }

  # La marca hay que ponerla después del último gancho: los temas escriben PROMPT en el
  # suyo. Ponerse el último no basta, porque powerlevel10k se recoloca al final en cada
  # prompt; así que, si el último gancho es de otro, se envuelve esa función (una vez)
  # para que marque el prompt al terminar, conservando su código de salida.
  _colibri_precmd() {
    local -i colibri_status=$?
    emulate -L zsh
    if (( ! ${+_colibri_ready} )); then
      typeset -g _colibri_ready=1
      _colibri_sel_setup
    fi
    # Solo si de verdad se ejecutó algo (un Enter vacío o un ^C no cuentan).
    typeset -g _colibri_result=
    if (( ${+_colibri_t0} )); then
      _colibri_result=";s=$colibri_status;t=$(( int((EPOCHREALTIME - _colibri_t0) * 1000) ))"
      unset _colibri_t0
    fi
    local last=$precmd_functions[-1]
    if [[ $last == _colibri_precmd ]]; then
      _colibri_mark
    elif [[ $last =~ '^[A-Za-z0-9_.:-]+$' ]] && (( ! $+functions[_colibri_wrapped_$last] )) &&
      functions -c -- $last _colibri_wrapped_$last 2>/dev/null; then
      eval "$last() { _colibri_wrapped_$last \"\$@\"; local -i ret=\$?; _colibri_mark; return ret }"
    fi
  }

  # ── Selección en la línea de comandos ──────────────────────────────────────────────
  # Shift+flechas, ⌘A y el ratón seleccionan texto del comando como en un editor: escribir
  # lo sustituye, borrar lo borra, ⌘C/⌘X lo copian o cortan. La selección es la «región»
  # de zle (marca + cursor), que el propio zsh pinta. Mientras hay selección manda el mapa
  # de teclas `colibri-sel`: lo que no sea de la selección la suelta y repite la tecla en
  # el mapa normal, así que ningún widget tuyo se entera.

  _colibri_sel_on() {
    REGION_ACTIVE=1
    if [[ $KEYMAP != colibri-sel ]]; then
      typeset -g _colibri_keymap=$KEYMAP
      zle -K colibri-sel
    fi
  }
  _colibri_sel_off() {
    REGION_ACTIVE=0
    [[ $KEYMAP == colibri-sel ]] && zle -K ${_colibri_keymap:-main}
    return 0
  }
  # Mueve el cursor con el widget dado, dejando la marca donde empezó la selección.
  _colibri_sel_move() {
    (( REGION_ACTIVE )) || MARK=$CURSOR
    _colibri_sel_on
    zle $1
  }
  _colibri_sel_left() { _colibri_sel_move .backward-char }
  _colibri_sel_right() { _colibri_sel_move .forward-char }
  _colibri_sel_word_left() { _colibri_sel_move .backward-word }
  _colibri_sel_word_right() { _colibri_sel_move .forward-word }
  # Entre las líneas de un comando de varias (Shift+↑/↓) y hasta su principio o su final
  # (Shift+⌥+↑/↓). Sin esto zle dejaba escrito el resto de la secuencia («[1;4A»).
  _colibri_sel_up() { _colibri_sel_move .up-line }
  _colibri_sel_down() { _colibri_sel_move .down-line }
  _colibri_sel_top() {
    (( REGION_ACTIVE )) || MARK=$CURSOR
    _colibri_sel_on
    CURSOR=0
  }
  _colibri_sel_bottom() {
    (( REGION_ACTIVE )) || MARK=$CURSOR
    _colibri_sel_on
    CURSOR=$#BUFFER
  }
  _colibri_sel_home() { _colibri_sel_move .beginning-of-line }
  _colibri_sel_end() { _colibri_sel_move .end-of-line }
  _colibri_sel_all() {
    (( $#BUFFER )) || return 0
    MARK=0
    CURSOR=$#BUFFER
    _colibri_sel_on
  }
  # Deja en REPLY el texto seleccionado y en `reply` sus dos extremos.
  _colibri_sel_text() {
    local -i a=MARK b=CURSOR
    (( a > b )) && { a=CURSOR; b=MARK }
    reply=($a $b)
    REPLY=${BUFFER[a+1,b]}
  }
  _colibri_sel_erase() {
    local REPLY
    local -a reply
    _colibri_sel_text
    BUFFER=${BUFFER[1,reply[1]]}${BUFFER[reply[2]+1,-1]}
    CURSOR=$reply[1]
    _colibri_sel_off
  }
  _colibri_sel_delete() { (( ! REGION_ACTIVE )) || _colibri_sel_erase }
  # Escribir (o pegar) con algo seleccionado: se borra y la tecla sigue su camino.
  _colibri_sel_replace() {
    _colibri_sel_erase
    zle -U "$KEYS"
  }
  # Cualquier otra tecla: se suelta la selección y la tecla se repite en el mapa normal.
  _colibri_sel_leave() {
    _colibri_sel_off
    zle -U "$KEYS"
  }
  _colibri_sel_copy() {
    (( REGION_ACTIVE )) || return 0
    local REPLY
    local -a reply
    _colibri_sel_text
    print -rn -- $REPLY | command pbcopy
  }
  _colibri_sel_cut() {
    (( REGION_ACTIVE )) || return 0
    _colibri_sel_copy
    _colibri_sel_erase
  }

  # Se instala en el primer prompt, después de tu .zshrc, para que este no pise las teclas.
  _colibri_sel_setup() {
    emulate -L zsh
    local w map
    for w in left right up down top bottom word_left word_right home end all delete replace \
      leave copy cut; do
      zle -N _colibri_sel_$w
    done
    bindkey -N colibri-sel
    bindkey -M colibri-sel -R '^@'-'^?' _colibri_sel_leave
    bindkey -M colibri-sel -R ' '-'~' _colibri_sel_replace
    bindkey -M colibri-sel -R '\M-^@'-'\M-^?' _colibri_sel_replace
    bindkey -M colibri-sel '^?' _colibri_sel_delete '^H' _colibri_sel_delete \
      '^[[3~' _colibri_sel_delete '^[[200~' _colibri_sel_replace
    for map in emacs viins colibri-sel; do
      bindkey -M $map '^[[1;2D' _colibri_sel_left '^[[1;2C' _colibri_sel_right \
        '^[[1;2A' _colibri_sel_up '^[[1;2B' _colibri_sel_down \
        '^[[1;4A' _colibri_sel_top '^[[1;4B' _colibri_sel_bottom \
        '^[[1;4D' _colibri_sel_word_left '^[[1;4C' _colibri_sel_word_right \
        '^[[1;2H' _colibri_sel_home '^[[1;2F' _colibri_sel_end \
        '^[[77;a~' _colibri_sel_all '^[[77;c~' _colibri_sel_copy '^[[77;x~' _colibri_sel_cut
    done
    # La selección, del color de selección del tema de Colibrí.
    if [[ -n ${COLIBRI_SELECTION-} ]]; then
      typeset -ga zle_highlight
      zle_highlight=(${zle_highlight:#region:*} region:bg=$COLIBRI_SELECTION)
    fi
  }

  zmodload zsh/datetime
  typeset -ga precmd_functions preexec_functions
  precmd_functions+=(_colibri_precmd)
  preexec_functions+=(_colibri_preexec)
fi
