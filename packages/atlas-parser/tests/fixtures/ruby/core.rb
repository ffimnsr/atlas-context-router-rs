require "json"
require_relative "helper"

module Demo
  class Runner
    include Logging
    extend Builders

    def helper
    end

    def run
      helper()
    end

    def self.build
      helper()
    end
  end
end
