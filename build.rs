// Ursa Minor - A Hypixel API proxy
// Copyright (C) 2023 Linnea Gräf
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use std::process::Command;
fn main() {
    let git_hash = if let Ok(git_hash) = std::env::var("GIT_HASH") {
        git_hash
    } else {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let git_hash = String::from_utf8(output.stdout).unwrap();
        git_hash
    };
    println!("cargo:rustc-env=GIT_HASH={}", git_hash);
}
