# fish completion for the Wraith compiler.
#
# Install by copying this file to ~/.config/fish/completions/wraith.fish.
# It can also be regenerated with `wraith --completions fish`.

# Don't fall back to completing every file; the positional argument is a .wr
# source file and is handled explicitly below.
complete -c wraith -f

complete -c wraith -s h -l help    -d 'Print help information'
complete -c wraith -s v -l version -d 'Print version information'

complete -c wraith -s c -l comments -x -a 'minimal normal verbose' \
    -d 'Comment verbosity in generated assembly'
complete -c wraith -s o -l out -x -a '(__fish_complete_directories)' \
    -d 'Write the .asm output to this directory'
complete -c wraith -l completions -x -a 'bash zsh fish' \
    -d 'Print a shell completion script'

complete -c wraith -a '(__fish_complete_suffix .wr)' -d 'Wraith source file'
