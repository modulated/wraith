# bash completion for the Wraith compiler.
#
# Install by sourcing this file from ~/.bashrc, or drop it into
# /usr/share/bash-completion/completions/wraith. It can also be regenerated
# with `wraith --completions bash`.

_wraith() {
    local cur prev opts
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"

    # Options that take an argument: complete the argument, not another flag.
    case "$prev" in
        -c|--comments)
            COMPREPLY=( $(compgen -W "minimal normal verbose" -- "$cur") )
            return 0
            ;;
        --cpu)
            COMPREPLY=( $(compgen -W "65c02 6502" -- "$cur") )
            return 0
            ;;
        -o|--out)
            COMPREPLY=( $(compgen -d -- "$cur") )
            return 0
            ;;
        --completions)
            COMPREPLY=( $(compgen -W "bash zsh fish" -- "$cur") )
            return 0
            ;;
    esac

    if [[ "$cur" == -* ]]; then
        opts="-h --help -v --version -c --comments --cpu -o --out --completions"
        COMPREPLY=( $(compgen -W "$opts" -- "$cur") )
        return 0
    fi

    # Positional argument: a .wr source file. Directories are offered too so
    # paths can be walked into.
    COMPREPLY=( $(compgen -f -X '!*.wr' -- "$cur") $(compgen -d -- "$cur") )
    return 0
}

complete -o filenames -F _wraith wraith
