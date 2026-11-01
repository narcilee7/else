class ElseBot < Formula
  desc "Managed personal agent runtime — you do what matters, else handles the rest"
  homepage "https://github.com/narcilee7/else"
  version "0.1.2"
  license "MIT"

  # 发布时由 scripts/update-homebrew-formula.py 填入 release 校验和。
  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-aarch64-apple-darwin.tar.gz"
      sha256 "22110b681d54704fe8e2d5da01eb4737d9659697e8aba0e63dc5552b3fb82430"
    else
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-x86_64-apple-darwin.tar.gz"
      sha256 "d523340d37fdf6fa36f93d09736d40caf2098f0a647b1588c8299d65f77e4473"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "24a99743aae954c7cebe02368340cd0d814a744297c00dcdcce044dab8856155"
    else
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "bfc782529fc0af775f7e2effe7a454185ed2ebf21527de6c3dee9b70898b0755"
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
