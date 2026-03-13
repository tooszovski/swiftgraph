class Swiftgraph < Formula
  desc "MCP server that builds code graphs from Swift projects"
  homepage "https://github.com/nicklama/swiftgraph"
  license "MIT"
  head "https://github.com/nicklama/swiftgraph.git", branch: "main"

  depends_on "rust" => :build
  depends_on :macos

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/swiftgraph-mcp")
    bin.install "target/release/swiftgraph" if File.exist?("target/release/swiftgraph")
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
    assert_match "Swift code graph MCP server", shell_output("#{bin}/swiftgraph --help")
  end
end
