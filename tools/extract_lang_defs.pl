#!/usr/bin/env perl
# Extract cloc's language definition tables into JSON.
#
# set_constants() is self-contained: it takes ten hashrefs and fills them in
# without touching any outer lexical. That makes it safe to lift the sub text
# out of the cloc script and eval it here, rather than re-typing 952 languages
# and 1017 extensions by hand.
#
# Usage: perl tools/extract_lang_defs.pl cloc data/languages.json

use warnings;
use strict;
use JSON::PP;

my $cloc_path = shift // 'cloc';
my $out_path  = shift // 'data/languages.json';

open my $fh, '<:encoding(UTF-8)', $cloc_path
    or die "Cannot read $cloc_path: $!\n";
my @src = <$fh>;
close $fh;

# ---- locate `sub set_constants { ... }` --------------------------------- {{{1
my ($start, $end);
for my $i (0 .. $#src) {
    $start = $i if !defined($start) && $src[$i] =~ /^sub set_constants \{/;
    if (defined($start) && $i > $start && $src[$i] =~ /^\} # 1\}\}\}/) {
        $end = $i;
        last;
    }
}
die "Could not locate set_constants in $cloc_path\n"
    unless defined $start and defined $end;

my $sub_text = join '', @src[$start .. $end];

# ---- locate `my %Extension_Collision = ( ... );` ------------------------ {{{1
my ($ec_start, $ec_end);
for my $i (0 .. $#src) {
    $ec_start = $i if !defined($ec_start) && $src[$i] =~ /^my %Extension_Collision = \(/;
    if (defined($ec_start) && $i >= $ec_start && $src[$i] =~ /^\);/) {
        $ec_end = $i;
        last;
    }
}
die "Could not locate %Extension_Collision in $cloc_path\n"
    unless defined $ec_start and defined $ec_end;

my $ec_text = join '', @src[$ec_start .. $ec_end];
# `my` inside a string eval declares a lexical scoped to the eval, which would
# leave our outer %Extension_Collision empty. Assign to the outer one instead.
$ec_text =~ s/^my \%Extension_Collision/\%Extension_Collision/;

# ---- evaluate ------------------------------------------------------------ {{{1
my (%Language_by_Extension, %Language_by_Script, %Language_by_File_Type,
    %Language_by_Prefix, %Filters_by_Language, %Not_Code_Extension,
    %Not_Code_Filename, %Scale_Factor, %Known_Binary_Archives,
    %EOL_Continuation_re, %Extension_Collision);

{
    no warnings 'redefine';
    # set_constants contains `die` entries as filter bodies for languages that
    # are never counted directly; those are data, not executed here.
    eval "$sub_text\n1;" or die "eval of set_constants failed: $@\n";
    eval "$ec_text\n1;"  or die "eval of \%Extension_Collision failed: $@\n";
}

set_constants(
    \%Language_by_Extension, \%Language_by_Script,  \%Language_by_File_Type,
    \%Language_by_Prefix,    \%Filters_by_Language, \%Not_Code_Extension,
    \%Not_Code_Filename,     \%Scale_Factor,        \%Known_Binary_Archives,
    \%EOL_Continuation_re,
);

# ---- normalise ----------------------------------------------------------- {{{1
# Filters are arrays-of-arrays in Perl: [ 'remove_matches', '^\s*//' ].
# Emit them as { filter => name, args => [...] } so the Rust side can match on
# a tagged enum instead of positional arrays.
my %filters;
for my $lang (keys %Filters_by_Language) {
    $filters{$lang} = [
        map {
            # A few filters carry a numeric flag as their last argument
            # (multiline_mode). Force every argument to a string so the JSON
            # is uniformly typed and the Rust side can use Vec<String>.
            my @args = map { "$_" } @{$_}[ 1 .. $#$_ ];
            { filter => $_->[0], args => \@args }
        } @{ $Filters_by_Language{$lang} }
    ];
}

# Not_Code_Extension / Not_Code_Filename are used as sets; drop the values.
my @not_code_ext      = sort keys %Not_Code_Extension;
my @not_code_filename = sort keys %Not_Code_Filename;
my @binary_archives   = sort keys %Known_Binary_Archives;

my $data = {
    language_by_extension => \%Language_by_Extension,
    language_by_script    => \%Language_by_Script,
    language_by_file_type => \%Language_by_File_Type,
    language_by_prefix    => \%Language_by_Prefix,
    filters_by_language   => \%filters,
    not_code_extension    => \@not_code_ext,
    not_code_filename     => \@not_code_filename,
    scale_factor          => \%Scale_Factor,
    known_binary_archives => \@binary_archives,
    eol_continuation_re   => \%EOL_Continuation_re,
    extension_collision   => \%Extension_Collision,
};

my $json = JSON::PP->new->canonical->pretty->utf8->encode($data);

open my $out, '>:raw', $out_path or die "Cannot write $out_path: $!\n";
print $out $json;
close $out;

printf STDERR "wrote %s\n", $out_path;
printf STDERR "  extensions        %5d\n", scalar keys %Language_by_Extension;
printf STDERR "  scripts           %5d\n", scalar keys %Language_by_Script;
printf STDERR "  file types        %5d\n", scalar keys %Language_by_File_Type;
printf STDERR "  prefixes          %5d\n", scalar keys %Language_by_Prefix;
printf STDERR "  languages         %5d\n", scalar keys %filters;
printf STDERR "  scale factors     %5d\n", scalar keys %Scale_Factor;
printf STDERR "  eol continuations %5d\n", scalar keys %EOL_Continuation_re;
printf STDERR "  not-code ext      %5d\n", scalar @not_code_ext;
printf STDERR "  not-code files    %5d\n", scalar @not_code_filename;
printf STDERR "  binary archives   %5d\n", scalar @binary_archives;
printf STDERR "  collisions        %5d\n", scalar keys %Extension_Collision;
