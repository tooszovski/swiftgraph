class Swiftgraph < Formula
  desc "MCP server that builds code graphs from Swift projects"
  homepage "https://github.com/tooszovski/swiftgraph"
  url "https://github.com/tooszovski/swiftgraph/archive/refs/tags/v0.5.0.tar.gz"
  sha256 "ee6ff06363f56447557182991c391ff4191d698f431b72421b5f4949c4a6f2ca"
  license "MIT"
  head "https://github.com/tooszovski/swiftgraph.git", branch: "main"

  depends_on "rust" => :build
  depends_on :macos

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/swiftgraph-mcp")
  end

  def caveats
    <<~EOS
      SwiftGraph works best with Xcode Index Store data.
      Build your project in Xcode first, then run:
        cd /path/to/ios-project
        swiftgraph init
        swiftgraph index

      To use as an MCP server with Claude Code, add to .mcp.json:
        {
          "mcpServers": {
            "swiftgraph": {
              "command": "#{bin}/swiftgraph",
              "args": ["serve", "--mcp"]
            }
          }
        }
    EOS
  end

  test do
    assert_match "swiftgraph", shell_output("#{bin}/swiftgraph --help")
  end
end
