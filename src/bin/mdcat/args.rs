// Copyright 2018-2020 Sebastian Wiesner <sebastian@swsnr.de>

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use clap::*;

pub(crate) fn app<'a, 'b>(default_columns: &'a str) -> App<'a, 'b> {
    app_from_crate!()
        // Merge flags and options w/ arguments together, include args in usage
        // string and show options in the order of declaration.
        .setting(AppSettings::UnifiedHelpMessage)
        .setting(AppSettings::DontCollapseArgsInUsage)
        .setting(AppSettings::DeriveDisplayOrder)
        .set_term_width(80)
        .after_help(
            "\
See 'man 1 mdcat' for more information.

Report issues to <https://github.com/lunaryorn/mdcat>.",
        )
        .arg(
            Arg::with_name("paginate")
                .short("p")
                .long("--paginate")
                .help("Paginate the output of mdcat with a pager like less."),
        )
        .arg(
            Arg::with_name("no_pager")
                .short("P")
                .long("--no-pager")
                .help("Do not page output.  Default if invoked as mdcat")
                .overrides_with("paginate"),
        )
        .arg(
            Arg::with_name("filenames")
                .multiple(true)
                .help("The file to read.  If - read from standard input instead")
                .default_value("-"),
        )
        .arg(
            Arg::with_name("no_colour")
                .short("c")
                .long("--no-colour")
                .aliases(&["nocolour", "no-color", "nocolor"])
                .help("Disable all colours and other styles."),
        )
        .arg(
            Arg::with_name("columns")
                .long("columns")
                .help("Maximum number of columns to use for output")
                .default_value(default_columns),
        )
        .arg(
            Arg::with_name("local_only")
                .short("l")
                .long("local")
                .help("Do not load remote resources like images"),
        )
        .arg(
            Arg::with_name("dump_events")
                .long("dump-events")
                .help("Dump Markdown parser events and exit")
                .hidden(true),
        )
        .arg(
            Arg::with_name("fail_fast")
                .long("fail")
                .help("Exit immediately if any error occurs processing an input file"),
        )
        .arg(
            Arg::with_name("detect_only")
                .long("detect-only")
                .help("Only detect the terminal type and exit")
                .hidden(true),
        )
        .arg(
            Arg::with_name("ansi_only")
                .long("ansi-only")
                .help("Limit to standard ANSI formatting")
                .conflicts_with("no_colour")
                .hidden(true),
        )
}
