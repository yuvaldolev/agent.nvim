if vim.g.loaded_agent_amp then
    return
end
vim.g.loaded_agent_amp = true

require("agent_amp").setup()
