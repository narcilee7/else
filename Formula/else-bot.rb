class ElseBot < Formula
  desc "Managed personal agent runtime — you do what matters, else handles the rest"
  homepage "https://github.com/narcilee7/else"
  version "0.1.1"
  license "MIT"

  # 发布时由 scripts/update-homebrew-formula.py 填入 release 校验和。
  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-aarch64-apple-darwin.tar.gz"
      sha256 "bc84a6ac336f191952f728bf4cc97c63387fd3b3df1b66964d7d8bc5327f0e72"
    else
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-x86_64-apple-darwin.tar.gz"
      sha256 "bc519251bfe8c657163a6f7262140ff04caef474f783238cb7d182c696c35e6a"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "470bb77d93de0e9fa7028494a8c6acc1b70fabf2c0c195419a1e45fd3f3d8d65"
    else
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "37713a9634bb6d3dbc51e45786c82e499161386da21f8d5bad6a69b217390d20"
    end
  end

  def install
    if File.exist?("else-bot")
      bin.install "else-bot"
    elsif File.exist?("else-cli")
      bin.install "else-cli" => "else-bot"
    else
      bin.install "else" => "else-bot"
    end
    bin.install_symlink "else-bot" => "else-cli"
    bin.install_symlink "else-bot" => "else"
  end

  def caveats
    <<~EOS
      else 的 Runtime 需要常驻，Connector 才能按计划同步、通知才能按时送达。

        else-bot onboard    准备 runtime 目录并注册后台服务
        else-bot doctor     检查运行状态
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/else-bot --version")
  end
end
