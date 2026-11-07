class ElseBot < Formula
  desc "Managed personal agent runtime — you do what matters, else handles the rest"
  homepage "https://github.com/narcilee7/else"
  version "0.1.3"
  license "MIT"

  # 发布时由 scripts/update-homebrew-formula.py 填入 release 校验和。
  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-aarch64-apple-darwin.tar.gz"
      sha256 "9b9d715262381270ee16921073f74d978ae190deb74af95ecbf21049ee9a8966"
    else
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-x86_64-apple-darwin.tar.gz"
      sha256 "f147947f48d71abc7094670dccb2378e38a1398c7b19cc042504c98e1f5d71c2"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "47f0f243d7ec4462a1cf45003ec57211197a9c80570ac7520cf9dbe5839aae45"
    else
      url "https://github.com/narcilee7/else/releases/download/v#{version}/else-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "6034317dc6944f081ae807534089f2ff34ba974e4d22ab6ea1f73e1e341a99d6"
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
